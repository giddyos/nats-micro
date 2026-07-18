use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Context;
use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro_shared::TransportError;
use serde::de::DeserializeOwned;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone)]
pub struct CapturedEvent {
    subject: String,
    payload: Bytes,
    headers: Option<HeaderMap>,
}

impl CapturedEvent {
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&HeaderMap> {
        self.headers.as_ref()
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventLog {
    inner: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl EventLog {
    pub fn record(&self, subject: &str, payload: Bytes, headers: Option<HeaderMap>) {
        self.events().push(CapturedEvent {
            subject: subject.to_owned(),
            payload,
            headers,
        });
    }

    #[must_use]
    pub fn all(&self) -> Vec<CapturedEvent> {
        self.events().clone()
    }

    #[must_use]
    pub fn subject(&self, subject: impl Into<String>) -> EventSelection {
        EventSelection {
            events: self.clone(),
            subject: subject.into(),
        }
    }

    pub fn clear(&self) {
        self.events().clear();
    }

    fn events(&self) -> std::sync::MutexGuard<'_, Vec<CapturedEvent>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Clone)]
pub struct EventSelection {
    events: EventLog,
    subject: String,
}

impl EventSelection {
    #[must_use]
    pub fn count(&self) -> usize {
        self.events
            .events()
            .iter()
            .filter(|event| event.subject == self.subject)
            .count()
    }

    pub fn assert_count(&self, expected: usize) {
        assert_eq!(
            self.count(),
            expected,
            "unexpected event count for subject `{}`",
            self.subject
        );
    }

    pub fn single(&self) -> anyhow::Result<CapturedEvent> {
        let matching: Vec<_> = self
            .events
            .events()
            .iter()
            .filter(|event| event.subject == self.subject)
            .cloned()
            .collect();
        anyhow::ensure!(
            matching.len() == 1,
            "expected one event for subject `{}`, found {}",
            self.subject,
            matching.len()
        );
        Ok(matching.into_iter().next().expect("length checked"))
    }

    pub fn single_json<T>(&self) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        let event = self.single()?;
        serde_json::from_slice(&event.payload).with_context(|| {
            format!(
                "failed to decode event on subject `{}` as JSON",
                self.subject
            )
        })
    }

    pub fn single_proto<T>(&self) -> anyhow::Result<T>
    where
        T: prost::Message + Default,
    {
        let event = self.single()?;
        T::decode(event.payload).with_context(|| {
            format!(
                "failed to decode event on subject `{}` as protobuf",
                self.subject
            )
        })
    }

    pub fn headers(&self) -> anyhow::Result<Option<HeaderMap>> {
        Ok(self.single()?.headers)
    }

    pub fn clear(&self) {
        self.events
            .events()
            .retain(|event| event.subject != self.subject);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultOperation {
    Request,
    Publish,
}

#[derive(Debug, Clone)]
struct PlannedFault {
    operation: FaultOperation,
    subject: String,
    error: TransportError,
}

#[derive(Debug, Clone, Default)]
pub struct FaultPlan {
    inner: Arc<Mutex<VecDeque<PlannedFault>>>,
}

impl FaultPlan {
    pub fn timeout_next(&self, subject: impl Into<String>) {
        self.push(
            FaultOperation::Request,
            subject.into(),
            TransportError::Timeout,
        );
    }

    pub fn no_responders_for(&self, subject: impl Into<String>) {
        self.push(
            FaultOperation::Request,
            subject.into(),
            TransportError::NoResponders,
        );
    }

    pub fn fail_next_publish(&self, subject: impl Into<String>, error: TransportError) {
        self.push(FaultOperation::Publish, subject.into(), error);
    }

    pub fn before_request(&self, subject: &str) -> Result<(), TransportError> {
        self.consume(FaultOperation::Request, subject)
    }

    pub fn before_publish(&self, subject: &str) -> Result<(), TransportError> {
        self.consume(FaultOperation::Publish, subject)
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        self.faults().len()
    }

    pub fn clear(&self) {
        self.faults().clear();
    }

    fn push(&self, operation: FaultOperation, subject: String, error: TransportError) {
        self.faults().push_back(PlannedFault {
            operation,
            subject,
            error,
        });
    }

    fn consume(&self, operation: FaultOperation, subject: &str) -> Result<(), TransportError> {
        let mut faults = self.faults();
        let Some(index) = faults
            .iter()
            .position(|fault| fault.operation == operation && fault.subject == subject)
        else {
            return Ok(());
        };
        let fault = faults
            .remove(index)
            .expect("fault index came from iteration");
        Err(fault.error)
    }

    fn faults(&self) -> std::sync::MutexGuard<'_, VecDeque<PlannedFault>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub fn init_test_tracing() {
    static INITIALIZED: OnceLock<()> = OnceLock::new();
    INITIALIZED.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_test_writer()
                    .without_time(),
            )
            .try_init();
    });
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use nats_micro_shared::TransportError;

    use super::{EventLog, FaultPlan};

    #[test]
    fn event_selections_decode_and_clear() {
        let events = EventLog::default();
        events.record("users.created", Bytes::from_static(br#"{"id":7}"#), None);
        events.subject("users.created").assert_count(1);
        let value: serde_json::Value = events.subject("users.created").single_json().unwrap();
        assert_eq!(value["id"], 7);
        events.subject("users.created").clear();
        assert!(events.all().is_empty());
    }

    #[test]
    fn faults_are_consumed_deterministically() {
        let faults = FaultPlan::default();
        faults.timeout_next("users.create");
        faults.no_responders_for("billing.charge");
        faults.fail_next_publish("users.created", TransportError::Disconnected);

        assert_eq!(faults.before_request("other"), Ok(()));
        assert_eq!(
            faults.before_request("users.create"),
            Err(TransportError::Timeout)
        );
        assert_eq!(
            faults.before_request("billing.charge"),
            Err(TransportError::NoResponders)
        );
        assert_eq!(
            faults.before_publish("users.created"),
            Err(TransportError::Disconnected)
        );
        assert_eq!(faults.pending(), 0);
    }
}
