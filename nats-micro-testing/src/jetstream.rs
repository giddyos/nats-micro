use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_nats::HeaderMap;
use bytes::Bytes;

use crate::EventLog;

#[derive(Debug, Clone)]
struct StreamDeclaration {
    name: String,
    subjects: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct JetStreamConfig {
    streams: Vec<StreamDeclaration>,
    dead_letters: BTreeMap<String, String>,
}

impl JetStreamConfig {
    pub fn stream<I, Subject>(&mut self, name: impl Into<String>, subjects: I)
    where
        I: IntoIterator<Item = Subject>,
        Subject: Into<String>,
    {
        self.streams.push(StreamDeclaration {
            name: name.into(),
            subjects: subjects.into_iter().map(Into::into).collect(),
        });
    }

    pub fn dead_letter(&mut self, durable: impl Into<String>, subject: impl Into<String>) {
        self.dead_letters.insert(durable.into(), subject.into());
    }
}

#[derive(Debug, Clone)]
pub struct SimulatedConsumerConfig {
    pub stream: String,
    pub durable: String,
    pub filter_subject: String,
    pub ack_wait: Duration,
    pub max_deliver: u64,
    pub backoff: Vec<Duration>,
}

#[derive(Debug, Clone)]
struct ScheduledDelivery {
    durable: String,
    subject: String,
    payload: Bytes,
    headers: Option<HeaderMap>,
    attempt: u64,
    due_at: Duration,
}

#[derive(Debug, Clone)]
pub struct SimulatedDelivery {
    scheduled: ScheduledDelivery,
}

impl SimulatedDelivery {
    #[must_use]
    pub fn durable(&self) -> &str {
        &self.scheduled.durable
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.scheduled.subject
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.scheduled.payload
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&HeaderMap> {
        self.scheduled.headers.as_ref()
    }

    #[must_use]
    pub const fn attempt(&self) -> u64 {
        self.scheduled.attempt
    }

    #[must_use]
    pub fn into_parts(self) -> (String, Bytes, Option<HeaderMap>) {
        (
            self.scheduled.subject,
            self.scheduled.payload,
            self.scheduled.headers,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedAction {
    Ack,
    Nack,
    NackAfter(Duration),
    Term,
}

#[derive(Debug)]
struct State {
    config: JetStreamConfig,
    consumers: BTreeMap<String, SimulatedConsumerConfig>,
    pending: VecDeque<ScheduledDelivery>,
    deliveries: BTreeMap<String, u64>,
    timeout_next: VecDeque<String>,
    now: Duration,
}

#[derive(Debug, Clone)]
pub struct JetStreamSimulator {
    inner: Arc<Mutex<State>>,
    events: EventLog,
}

impl JetStreamSimulator {
    #[must_use]
    pub fn new(config: JetStreamConfig, events: EventLog) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                config,
                consumers: BTreeMap::new(),
                pending: VecDeque::new(),
                deliveries: BTreeMap::new(),
                timeout_next: VecDeque::new(),
                now: Duration::ZERO,
            })),
            events,
        }
    }

    pub fn register_consumer(&self, consumer: SimulatedConsumerConfig) -> anyhow::Result<()> {
        let mut state = self.state();
        anyhow::ensure!(
            state
                .config
                .streams
                .iter()
                .any(|stream| stream.name == consumer.stream),
            "simulated stream `{}` is not declared for durable `{}`",
            consumer.stream,
            consumer.durable
        );
        anyhow::ensure!(
            !state.consumers.contains_key(&consumer.durable),
            "simulated durable `{}` is declared more than once",
            consumer.durable
        );
        state.consumers.insert(consumer.durable.clone(), consumer);
        Ok(())
    }

    pub fn publish(
        &self,
        subject: &str,
        payload: &Bytes,
        headers: Option<&HeaderMap>,
    ) -> anyhow::Result<()> {
        let mut state = self.state();
        let matching_streams: Vec<_> = state
            .config
            .streams
            .iter()
            .filter(|stream| {
                stream
                    .subjects
                    .iter()
                    .any(|pattern| subject_matches(pattern, subject))
            })
            .map(|stream| stream.name.clone())
            .collect();
        anyhow::ensure!(
            !matching_streams.is_empty(),
            "no simulated stream accepts subject `{subject}`"
        );
        let consumers: Vec<_> = state
            .consumers
            .values()
            .filter(|consumer| {
                matching_streams.contains(&consumer.stream)
                    && subject_matches(&consumer.filter_subject, subject)
            })
            .map(|consumer| consumer.durable.clone())
            .collect();
        let now = state.now;
        for durable in consumers {
            state.pending.push_back(ScheduledDelivery {
                durable,
                subject: subject.to_owned(),
                payload: payload.clone(),
                headers: headers.cloned(),
                attempt: 1,
                due_at: now,
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn next_due(&self) -> Option<SimulatedDelivery> {
        let mut state = self.state();
        let index = state
            .pending
            .iter()
            .position(|delivery| delivery.due_at <= state.now)?;
        let scheduled = state.pending.remove(index)?;
        *state
            .deliveries
            .entry(scheduled.durable.clone())
            .or_default() += 1;
        Some(SimulatedDelivery { scheduled })
    }

    pub fn complete(&self, delivery: SimulatedDelivery, action: SimulatedAction) {
        match action {
            SimulatedAction::Ack | SimulatedAction::Term => {}
            SimulatedAction::Nack => self.retry(delivery, Duration::ZERO),
            SimulatedAction::NackAfter(delay) => self.retry(delivery, delay),
        }
    }

    pub fn complete_timeout(&self, delivery: SimulatedDelivery) {
        let delay = {
            let state = self.state();
            let consumer = state
                .consumers
                .get(delivery.durable())
                .expect("delivery durable is registered");
            let backoff_index =
                usize::try_from(delivery.attempt().saturating_sub(1)).unwrap_or(usize::MAX);
            consumer
                .backoff
                .get(backoff_index)
                .copied()
                .unwrap_or(consumer.ack_wait)
        };
        self.retry(delivery, delay);
    }

    pub fn timeout_next(&self, durable: impl Into<String>) {
        self.state().timeout_next.push_back(durable.into());
    }

    #[must_use]
    pub fn consume_timeout(&self, durable: &str) -> bool {
        let mut state = self.state();
        let Some(index) = state
            .timeout_next
            .iter()
            .position(|planned| planned == durable)
        else {
            return false;
        };
        state.timeout_next.remove(index);
        true
    }

    #[must_use]
    pub fn deliveries(&self, durable: &str) -> u64 {
        self.state()
            .deliveries
            .get(durable)
            .copied()
            .unwrap_or_default()
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        self.state().pending.len()
    }

    #[must_use]
    pub fn clock(&self) -> ManualClock {
        ManualClock {
            simulator: self.clone(),
        }
    }

    fn retry(&self, mut delivery: SimulatedDelivery, delay: Duration) {
        let mut state = self.state();
        let consumer = state
            .consumers
            .get(delivery.durable())
            .expect("delivery durable is registered");
        if delivery.attempt() >= consumer.max_deliver {
            if let Some(subject) = state.config.dead_letters.get(delivery.durable()) {
                self.events.record(
                    subject,
                    delivery.scheduled.payload,
                    delivery.scheduled.headers,
                );
            }
            return;
        }
        delivery.scheduled.attempt += 1;
        delivery.scheduled.due_at = state.now.saturating_add(delay);
        state.pending.push_back(delivery.scheduled);
    }

    fn advance_internal(&self, duration: Duration) {
        let mut state = self.state();
        state.now = state.now.saturating_add(duration);
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Clone)]
pub struct ManualClock {
    simulator: JetStreamSimulator,
}

impl ManualClock {
    pub async fn advance(&self, duration: Duration) {
        self.simulator.advance_internal(duration);
        tokio::time::advance(duration).await;
    }
}

fn subject_matches(pattern: &str, subject: &str) -> bool {
    let mut pattern = pattern.split('.');
    let mut subject = subject.split('.');
    loop {
        match (pattern.next(), subject.next()) {
            (Some(">"), Some(_)) => return pattern.next().is_none(),
            (Some("*"), Some(_)) => {}
            (Some(expected), Some(actual)) if expected == actual => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}
