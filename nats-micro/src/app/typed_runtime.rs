#![cfg_attr(not(feature = "telemetry"), allow(unused_variables))]

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use async_nats::service::ServiceExt;
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::{sync::watch, task::JoinHandle, time::Instant};

use super::{AppConfig, WorkerFailurePolicy};
#[cfg(feature = "telemetry")]
use crate::runtime::run_consumer_with_telemetry;
#[cfg(all(feature = "encryption", not(feature = "telemetry")))]
use crate::runtime::run_encrypted_request_endpoint_with_panic_policy;
#[cfg(all(feature = "encryption", feature = "telemetry"))]
use crate::runtime::run_encrypted_request_endpoint_with_telemetry;
#[cfg(not(feature = "telemetry"))]
use crate::runtime::{run_consumer_with_panic_policy, run_request_endpoint_with_panic_policy};
use crate::{
    ConsumerHandler, RequestEndpoint, ShutdownState, SubscriptionHandler,
    runtime::run_subscription_with_panic_policy,
};
#[cfg(feature = "telemetry")]
use crate::{MetricName, runtime::run_request_endpoint_with_telemetry, telemetry::Telemetry};

pub type StartError = anyhow::Error;
pub(crate) type ShutdownHook =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send>> + Send + Sync>;

pub struct Runtime<S> {
    client: crate::NatsClient,
    state: Arc<S>,
    config: AppConfig,
    shutdown_tx: watch::Sender<ShutdownState>,
    shutdown_rx: watch::Receiver<ShutdownState>,
    services: Vec<Arc<async_nats::service::Service>>,
    workers: Vec<JoinHandle<anyhow::Result<()>>>,
    shutdown_hook: Option<ShutdownHook>,
    #[cfg(feature = "encryption")]
    encryption: Option<Arc<crate::ServiceKeyPair>>,
    #[cfg(feature = "telemetry")]
    telemetry: Option<Telemetry>,
}

impl<S> Runtime<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(client: crate::NatsClient, state: Arc<S>, config: AppConfig) -> Self {
        let drain_timeout = Some(config.shutdown_drain_timeout);
        let (shutdown_tx, shutdown_rx) = watch::channel(ShutdownState::running(drain_timeout));
        Self {
            client,
            state,
            config,
            shutdown_tx,
            shutdown_rx,
            services: Vec::new(),
            workers: Vec::new(),
            shutdown_hook: None,
            #[cfg(feature = "encryption")]
            encryption: None,
            #[cfg(feature = "telemetry")]
            telemetry: None,
        }
    }

    pub(crate) fn set_shutdown_hook(&mut self, hook: Option<ShutdownHook>) {
        self.shutdown_hook = hook;
    }

    #[cfg(feature = "encryption")]
    pub(crate) fn set_encryption_key(&mut self, keypair: Option<Arc<crate::ServiceKeyPair>>) {
        self.encryption = keypair;
    }

    #[cfg(feature = "telemetry")]
    pub(crate) fn set_telemetry_layer(&mut self, layer: Option<Arc<dyn crate::TelemetryLayer>>) {
        self.telemetry = layer.map(|layer| Telemetry::new(layer, self.config.telemetry));
    }

    pub async fn start_service(&mut self, spec: crate::ServiceSpec) -> Result<(), StartError> {
        let service = self
            .client
            .service_builder()
            .description(spec.description)
            .start(spec.name, spec.version)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        self.services.push(Arc::new(service));
        Ok(())
    }

    pub async fn spawn_request<E>(&mut self) -> Result<(), StartError>
    where
        E: RequestEndpoint<S>,
    {
        let concurrency = effective_concurrency(&self.config, E::SPEC.concurrency);
        let service = Arc::clone(
            self.services
                .last()
                .ok_or_else(|| anyhow::anyhow!("request endpoint started before its service"))?,
        );
        let endpoint = register_request(&service, &E::SPEC).await?;
        let state = Arc::clone(&self.state);
        let shutdown = self.shutdown_rx.clone();
        let panic_policy = self.config.handler_panic;
        let failure_policy = self.config.worker_failure;
        #[cfg(feature = "telemetry")]
        let telemetry = self.telemetry.clone();
        self.workers.push(tokio::spawn(async move {
            supervise_request::<S, E>(
                state,
                service,
                endpoint,
                concurrency,
                panic_policy,
                failure_policy,
                #[cfg(feature = "telemetry")]
                telemetry,
                shutdown,
            )
            .await
        }));
        Ok(())
    }

    #[cfg(feature = "encryption")]
    pub async fn spawn_encrypted_request<E>(&mut self) -> Result<(), StartError>
    where
        E: crate::EncryptedRequestEndpoint<S>,
    {
        let concurrency = effective_concurrency(&self.config, E::SPEC.concurrency);
        let service = Arc::clone(self.services.last().ok_or_else(|| {
            anyhow::anyhow!("encrypted request endpoint started before its service")
        })?);
        let endpoint = register_request(&service, &E::SPEC).await?;
        let keypair = self.encryption.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "service `{}` declares encrypted operations but App has no encryption key",
                E::SPEC.rust_name
            )
        })?;
        let state = Arc::clone(&self.state);
        let shutdown = self.shutdown_rx.clone();
        let panic_policy = self.config.handler_panic;
        let failure_policy = self.config.worker_failure;
        #[cfg(feature = "telemetry")]
        let telemetry = self.telemetry.clone();
        self.workers.push(tokio::spawn(async move {
            supervise_encrypted_request::<S, E>(
                state,
                keypair,
                service,
                endpoint,
                concurrency,
                panic_policy,
                failure_policy,
                #[cfg(feature = "telemetry")]
                telemetry,
                shutdown,
            )
            .await
        }));
        Ok(())
    }

    pub async fn spawn_subscription<H>(&mut self) -> Result<(), StartError>
    where
        H: SubscriptionHandler<S>,
    {
        let concurrency = effective_concurrency(&self.config, H::SPEC.concurrency);
        let subscriber = if let Some(queue) = H::SPEC.queue_group {
            self.client
                .queue_subscribe(H::SPEC.subject, queue.to_owned())
                .await?
        } else {
            self.client.subscribe(H::SPEC.subject).await?
        };
        let state = Arc::clone(&self.state);
        let shutdown = self.shutdown_rx.clone();
        let panic_policy = self.config.handler_panic;
        let failure_policy = self.config.worker_failure;
        let client = self.client.clone();
        #[cfg(feature = "telemetry")]
        let telemetry = self.telemetry.clone();
        self.workers.push(tokio::spawn(async move {
            supervise_subscription::<S, H>(
                state,
                client,
                subscriber,
                concurrency,
                panic_policy,
                failure_policy,
                #[cfg(feature = "telemetry")]
                telemetry,
                shutdown,
            )
            .await
        }));
        Ok(())
    }

    pub async fn spawn_consumer<C>(&mut self) -> Result<(), StartError>
    where
        C: ConsumerHandler<S>,
    {
        let concurrency = effective_concurrency(&self.config, C::SPEC.concurrency);
        let messages = bind_consumer::<S, C>(&self.client).await?;
        let state = Arc::clone(&self.state);
        let shutdown = self.shutdown_rx.clone();
        let panic_policy = self.config.handler_panic;
        let failure_policy = self.config.worker_failure;
        let client = self.client.clone();
        #[cfg(feature = "telemetry")]
        let telemetry = self.telemetry.clone();
        self.workers.push(tokio::spawn(async move {
            supervise_consumer::<S, C>(
                state,
                client,
                messages,
                concurrency,
                panic_policy,
                failure_policy,
                #[cfg(feature = "telemetry")]
                telemetry,
                shutdown,
            )
            .await
        }));
        Ok(())
    }

    #[must_use]
    pub fn shutdown_sender(&self) -> watch::Sender<ShutdownState> {
        self.shutdown_tx.clone()
    }

    pub(crate) async fn shutdown_after_failed_start(self) -> anyhow::Result<()> {
        let _ = self.shutdown_tx.send(ShutdownState::requested(Some(
            self.config.shutdown_drain_timeout,
        )));
        self.run().await
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut workers: FuturesUnordered<_> = self.workers.drain(..).collect();
        let mut shutdown = self.shutdown_rx.clone();
        let mut first_failure = None;

        loop {
            if shutdown.borrow().is_requested() {
                break;
            }
            if workers.is_empty() {
                if shutdown.changed().await.is_err() || shutdown.borrow().is_requested() {
                    break;
                }
                continue;
            }
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || shutdown.borrow().is_requested() {
                        break;
                    }
                }
                completed = workers.next() => {
                    let Some(completed) = completed else {
                        break;
                    };
                    if shutdown.borrow().is_requested() {
                        break;
                    }
                    let failure = match completed {
                        Ok(Ok(())) => anyhow::anyhow!("static worker exited unexpectedly"),
                        Ok(Err(error)) => error,
                        Err(error) => anyhow::anyhow!("static worker task failed: {error}"),
                    };
                    match self.config.worker_failure {
                        WorkerFailurePolicy::Ignore => {
                            crate::trace::warn!(%failure, "static worker failed; ignoring by policy");
                        }
                        WorkerFailurePolicy::ShutdownApp | WorkerFailurePolicy::Restart { .. } => {
                            first_failure = Some(failure);
                            let _ = self.shutdown_tx.send(ShutdownState::requested(
                                Some(self.config.shutdown_drain_timeout),
                            ));
                            break;
                        }
                    }
                }
            }
        }

        if let Some(hook) = self.shutdown_hook
            && let Err(error) = hook().await
            && first_failure.is_none()
        {
            first_failure = Some(error);
        }

        #[cfg(feature = "telemetry")]
        let drain_started = Instant::now();
        let deadline = Instant::now() + self.config.shutdown_drain_timeout;
        let _ = self.shutdown_tx.send(ShutdownState::draining(
            Some(self.config.shutdown_drain_timeout),
            deadline,
        ));
        drain_workers(&mut workers, deadline, &mut first_failure).await;
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.application(
                MetricName::ShutdownDrainDuration,
                Some(drain_started.elapsed()),
            );
        }

        for service in self.services {
            match Arc::try_unwrap(service) {
                Ok(service) => {
                    if let Err(error) = service.stop().await {
                        crate::trace::debug!(%error, "service registration was already stopped");
                    }
                }
                Err(_) => {
                    first_failure.get_or_insert_with(|| {
                        anyhow::anyhow!(
                            "service registration remained referenced after worker drain"
                        )
                    });
                }
            }
        }
        if let Err(error) = self.client.drain().await {
            first_failure.get_or_insert_with(|| anyhow::anyhow!(error.to_string()));
        }

        if let Some(error) = first_failure {
            Err(error)
        } else {
            Ok(())
        }
    }
}

async fn drain_workers(
    workers: &mut FuturesUnordered<JoinHandle<anyhow::Result<()>>>,
    deadline: Instant,
    first_failure: &mut Option<anyhow::Error>,
) {
    while !workers.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timed_out = remaining.is_zero()
            || tokio::time::timeout(remaining, workers.next())
                .await
                .is_err();
        if timed_out {
            for worker in &*workers {
                worker.abort();
            }
            while workers.next().await.is_some() {}
            first_failure
                .get_or_insert_with(|| anyhow::anyhow!("timed out draining static workers"));
            return;
        }
    }
}

async fn register_request(
    service: &async_nats::service::Service,
    spec: &crate::OperationSpec,
) -> Result<async_nats::service::endpoint::Endpoint, StartError> {
    let mut builder = service.endpoint_builder().name(spec.rust_name);
    if let Some(queue) = spec.queue_group {
        builder = builder.queue_group(queue);
    }
    builder
        .add(spec.subject)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[allow(clippy::too_many_arguments)]
async fn supervise_request<S, E>(
    state: Arc<S>,
    service: Arc<async_nats::service::Service>,
    mut endpoint: async_nats::service::endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    failure_policy: WorkerFailurePolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    let mut restarts = RestartTracker::new(failure_policy);
    loop {
        #[cfg(feature = "telemetry")]
        let result = run_request_endpoint_with_telemetry::<S, E>(
            Arc::clone(&state),
            endpoint,
            concurrency,
            panic_policy,
            telemetry.clone(),
            shutdown.clone(),
        )
        .await;
        #[cfg(not(feature = "telemetry"))]
        let result = run_request_endpoint_with_panic_policy::<S, E>(
            Arc::clone(&state),
            endpoint,
            concurrency,
            panic_policy,
            shutdown.clone(),
        )
        .await;
        if shutdown.borrow().is_requested() {
            return Ok(());
        }
        let error = result
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("request worker exited unexpectedly"));
        let Some(delay) = restarts.next_delay() else {
            return Err(error);
        };
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.operation(
                MetricName::WorkerRestarts,
                1,
                None,
                &E::SPEC,
                None,
                None,
                None,
            );
        }
        crate::trace::warn!(%error, ?delay, endpoint = E::SPEC.subject, "restarting request worker");
        if wait_backoff(delay, &mut shutdown).await {
            return Ok(());
        }
        endpoint = register_request(&service, &E::SPEC).await?;
    }
}

#[cfg(feature = "encryption")]
#[allow(clippy::too_many_arguments)]
async fn supervise_encrypted_request<S, E>(
    state: Arc<S>,
    keypair: Arc<crate::ServiceKeyPair>,
    service: Arc<async_nats::service::Service>,
    mut endpoint: async_nats::service::endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    failure_policy: WorkerFailurePolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    let mut restarts = RestartTracker::new(failure_policy);
    loop {
        #[cfg(feature = "telemetry")]
        let result = run_encrypted_request_endpoint_with_telemetry::<S, E>(
            Arc::clone(&state),
            Arc::clone(&keypair),
            endpoint,
            concurrency,
            panic_policy,
            telemetry.clone(),
            shutdown.clone(),
        )
        .await;
        #[cfg(not(feature = "telemetry"))]
        let result = run_encrypted_request_endpoint_with_panic_policy::<S, E>(
            Arc::clone(&state),
            Arc::clone(&keypair),
            endpoint,
            concurrency,
            panic_policy,
            shutdown.clone(),
        )
        .await;
        if shutdown.borrow().is_requested() {
            return Ok(());
        }
        let error = result
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("encrypted request worker exited unexpectedly"));
        let Some(delay) = restarts.next_delay() else {
            return Err(error);
        };
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.operation(
                MetricName::WorkerRestarts,
                1,
                None,
                &E::SPEC,
                None,
                None,
                None,
            );
        }
        crate::trace::warn!(
            %error,
            ?delay,
            endpoint = E::SPEC.subject,
            "restarting encrypted request worker"
        );
        if wait_backoff(delay, &mut shutdown).await {
            return Ok(());
        }
        endpoint = register_request(&service, &E::SPEC).await?;
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise_subscription<S, H>(
    state: Arc<S>,
    client: crate::NatsClient,
    mut subscriber: async_nats::Subscriber,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    failure_policy: WorkerFailurePolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    H: SubscriptionHandler<S>,
{
    let mut restarts = RestartTracker::new(failure_policy);
    loop {
        let result = run_subscription_with_panic_policy::<S, H>(
            Arc::clone(&state),
            subscriber,
            concurrency,
            panic_policy,
            shutdown.clone(),
        )
        .await;
        if shutdown.borrow().is_requested() {
            return Ok(());
        }
        let error = result
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("subscription worker exited unexpectedly"));
        let Some(delay) = restarts.next_delay() else {
            return Err(error);
        };
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.operation(
                MetricName::WorkerRestarts,
                1,
                None,
                &H::SPEC,
                None,
                None,
                None,
            );
        }
        crate::trace::warn!(%error, ?delay, subject = H::SPEC.subject, "restarting subscription worker");
        if wait_backoff(delay, &mut shutdown).await {
            return Ok(());
        }
        subscriber = if let Some(queue) = H::SPEC.queue_group {
            client
                .queue_subscribe(H::SPEC.subject, queue.to_owned())
                .await?
        } else {
            client.subscribe(H::SPEC.subject).await?
        };
    }
}

async fn bind_consumer<S, C>(
    client: &crate::NatsClient,
) -> Result<async_nats::jetstream::consumer::push::Messages, StartError>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    let jetstream = async_nats::jetstream::new(client.clone());
    let stream = jetstream.get_stream(C::SPEC.stream).await?;
    let consumer = stream
        .get_or_create_consumer(
            C::SPEC.durable,
            async_nats::jetstream::consumer::push::Config {
                durable_name: Some(C::SPEC.durable.to_owned()),
                deliver_subject: client.new_inbox(),
                filter_subject: C::SPEC.filter_subject.to_owned(),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_millis(C::SPEC.ack_wait_ms),
                max_deliver: C::SPEC.max_deliver,
                backoff: C::SPEC
                    .backoff_ms
                    .iter()
                    .copied()
                    .map(Duration::from_millis)
                    .collect(),
                ..Default::default()
            },
        )
        .await?;
    Ok(consumer.messages().await?)
}

#[allow(clippy::too_many_arguments)]
async fn supervise_consumer<S, C>(
    state: Arc<S>,
    client: crate::NatsClient,
    mut messages: async_nats::jetstream::consumer::push::Messages,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    failure_policy: WorkerFailurePolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    let mut restarts = RestartTracker::new(failure_policy);
    loop {
        #[cfg(feature = "telemetry")]
        let result = run_consumer_with_telemetry::<S, C>(
            Arc::clone(&state),
            messages,
            concurrency,
            panic_policy,
            telemetry.clone(),
            shutdown.clone(),
        )
        .await;
        #[cfg(not(feature = "telemetry"))]
        let result = run_consumer_with_panic_policy::<S, C>(
            Arc::clone(&state),
            messages,
            concurrency,
            panic_policy,
            shutdown.clone(),
        )
        .await;
        if shutdown.borrow().is_requested() {
            return Ok(());
        }
        let error = result
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("consumer worker exited unexpectedly"));
        let Some(delay) = restarts.next_delay() else {
            return Err(error);
        };
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.consumer(
                MetricName::WorkerRestarts,
                1,
                None,
                &C::SPEC,
                None,
                None,
                None,
            );
        }
        crate::trace::warn!(%error, ?delay, durable = C::SPEC.durable, "restarting consumer worker");
        if wait_backoff(delay, &mut shutdown).await {
            return Ok(());
        }
        messages = bind_consumer::<S, C>(&client).await?;
    }
}

struct RestartTracker {
    policy: WorkerFailurePolicy,
    window_started: Instant,
    restarts: u32,
    next_backoff: Duration,
}

impl RestartTracker {
    fn new(policy: WorkerFailurePolicy) -> Self {
        let next_backoff = match policy {
            WorkerFailurePolicy::Restart {
                initial_backoff, ..
            } => initial_backoff,
            WorkerFailurePolicy::Ignore | WorkerFailurePolicy::ShutdownApp => Duration::ZERO,
        };
        Self {
            policy,
            window_started: Instant::now(),
            restarts: 0,
            next_backoff,
        }
    }

    fn next_delay(&mut self) -> Option<Duration> {
        let WorkerFailurePolicy::Restart {
            initial_backoff,
            max_backoff,
            max_restarts,
            window,
        } = self.policy
        else {
            return None;
        };
        if self.window_started.elapsed() > window {
            self.window_started = Instant::now();
            self.restarts = 0;
            self.next_backoff = initial_backoff;
        }
        if self.restarts >= max_restarts {
            return None;
        }
        self.restarts += 1;
        let delay = self.next_backoff;
        self.next_backoff = self.next_backoff.saturating_mul(2).min(max_backoff);
        Some(delay)
    }
}

async fn wait_backoff(delay: Duration, shutdown: &mut watch::Receiver<ShutdownState>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(delay) => false,
        changed = shutdown.changed() => {
            changed.is_err() || shutdown.borrow().is_requested()
        }
    }
}

fn effective_concurrency(config: &AppConfig, declared: usize) -> usize {
    let requested = if declared == 0 {
        config.default_concurrency
    } else {
        declared
    };
    requested.min(config.max_concurrency)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{RestartTracker, effective_concurrency};
    use crate::{AppConfig, Profile, WorkerFailurePolicy};

    #[test]
    fn profile_maximum_caps_declared_worker_concurrency() {
        let config = AppConfig::for_profile(Profile::Test);
        assert_eq!(effective_concurrency(&config, 256), 32);
        assert_eq!(effective_concurrency(&config, 8), 8);
        assert_eq!(effective_concurrency(&config, 0), 4);
    }

    #[test]
    fn restart_policy_applies_bounded_exponential_backoff() {
        let mut tracker = RestartTracker::new(WorkerFailurePolicy::Restart {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(250),
            max_restarts: 4,
            window: Duration::from_mins(1),
        });

        assert_eq!(tracker.next_delay(), Some(Duration::from_millis(100)));
        assert_eq!(tracker.next_delay(), Some(Duration::from_millis(200)));
        assert_eq!(tracker.next_delay(), Some(Duration::from_millis(250)));
        assert_eq!(tracker.next_delay(), Some(Duration::from_millis(250)));
        assert_eq!(tracker.next_delay(), None);
    }

    #[test]
    fn non_restart_policies_have_no_restart_budget() {
        assert_eq!(
            RestartTracker::new(WorkerFailurePolicy::ShutdownApp).next_delay(),
            None
        );
        assert_eq!(
            RestartTracker::new(WorkerFailurePolicy::Ignore).next_delay(),
            None
        );
    }
}
