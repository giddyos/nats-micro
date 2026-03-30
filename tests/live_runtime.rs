use std::{
    collections::BTreeSet,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use futures::StreamExt;
use nats_micro::{
    ConsumerDefinition, EndpointDefinition, NatsApp, NatsErrorResponse, Payload, ServiceDefinition,
    ServiceMetadata, ShutdownSignal, State, WorkerFailurePolicy, async_nats, service,
    service_handlers,
};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::timeout,
};
use uuid::Uuid;

#[service(name = "live-queue-alpha")]
struct LiveQueueAlphaService;

#[service_handlers]
impl LiveQueueAlphaService {
    #[endpoint(subject = "jobs", group = "live", queue_group = "alpha-live-workers")]
    async fn jobs() -> Result<&'static str, NatsErrorResponse> {
        Ok("alpha")
    }
}

#[service(name = "live-queue-beta")]
struct LiveQueueBetaService;

#[service_handlers]
impl LiveQueueBetaService {
    #[endpoint(subject = "jobs", group = "live", queue_group = "beta-live-workers")]
    async fn jobs() -> Result<&'static str, NatsErrorResponse> {
        Ok("beta")
    }
}

#[service(name = "live-supervision")]
struct LiveSupervisionService;

#[service_handlers]
impl LiveSupervisionService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }
}

#[derive(Clone, Default)]
struct ConsumerProbe {
    hits: Arc<AtomicUsize>,
    processed: Arc<Notify>,
}

impl ConsumerProbe {
    fn record(&self) {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.processed.notify_waiters();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShutdownObservation {
    requested: bool,
    drain_timeout: Option<Duration>,
    deadline_present: bool,
}

#[derive(Clone, Default)]
struct ShutdownProbe {
    started: Arc<Notify>,
    observation: Arc<Mutex<Option<ShutdownObservation>>>,
}

impl ShutdownProbe {
    fn mark_started(&self) {
        self.started.notify_waiters();
    }

    fn record(&self, shutdown: &ShutdownSignal) {
        *self.observation.lock().unwrap() = Some(ShutdownObservation {
            requested: shutdown.is_requested(),
            drain_timeout: shutdown.drain_timeout(),
            deadline_present: shutdown.deadline().is_some(),
        });
    }

    fn observation(&self) -> ShutdownObservation {
        self.observation
            .lock()
            .unwrap()
            .clone()
            .expect("shutdown observation should be recorded")
    }
}

#[service(name = "live-consumer-flow")]
struct LiveConsumerFlowService;

#[service_handlers]
impl LiveConsumerFlowService {
    #[consumer(
        stream = "LIVE_STREAM",
        durable = "LIVE_DURABLE",
        config = nats_micro::ConsumerConfig {
            ack_wait: std::time::Duration::from_secs(1),
            ..Default::default()
        }
    )]
    async fn jobs(
        _payload: Payload<String>,
        probe: State<ConsumerProbe>,
    ) -> Result<(), NatsErrorResponse> {
        probe.record();
        Ok(())
    }
}

#[service(name = "live-shutdown-endpoint")]
struct LiveShutdownEndpointService;

#[service_handlers]
impl LiveShutdownEndpointService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[endpoint(subject = "cleanup", group = "live")]
    async fn cleanup(
        mut shutdown: ShutdownSignal,
        probe: State<ShutdownProbe>,
    ) -> Result<&'static str, NatsErrorResponse> {
        probe.mark_started();
        shutdown.wait_for_shutdown().await;
        probe.record(&shutdown);
        Ok("cleaned")
    }
}

#[service(name = "live-shutdown-consumer")]
struct LiveShutdownConsumerService;

#[service_handlers]
impl LiveShutdownConsumerService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[consumer(
        stream = "LIVE_SHUTDOWN_STREAM",
        durable = "LIVE_SHUTDOWN_DURABLE",
        config = nats_micro::ConsumerConfig {
            ack_wait: std::time::Duration::from_secs(1),
            ..Default::default()
        }
    )]
    async fn jobs(
        _payload: Payload<String>,
        mut shutdown: ShutdownSignal,
        probe: State<ShutdownProbe>,
    ) -> Result<(), NatsErrorResponse> {
        probe.mark_started();
        shutdown.wait_for_shutdown().await;
        probe.record(&shutdown);
        Ok(())
    }
}

#[test]
fn macro_generated_handlers_only_enable_shutdown_support_when_needed() {
    assert!(!LiveQueueAlphaService::jobs_endpoint()
        .handler
        .requires_shutdown_signal());
    assert!(!LiveConsumerFlowService::jobs_consumer()
        .handler
        .requires_shutdown_signal());
    assert!(LiveShutdownEndpointService::cleanup_endpoint()
        .handler
        .requires_shutdown_signal());
    assert!(LiveShutdownConsumerService::jobs_consumer()
        .handler
        .requires_shutdown_signal());
}

#[tokio::test]
async fn live_queue_groups_receive_independent_copies() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let unique = unique_name("queue-groups");
    let group = format!("live.{unique}");
    let full_subject = format!("{group}.jobs");

    let mut alpha_endpoint = LiveQueueAlphaService::jobs_endpoint();
    alpha_endpoint.group = group.clone();

    let mut beta_endpoint = LiveQueueBetaService::jobs_endpoint();
    beta_endpoint.group = group.clone();

    let (alpha_shutdown, alpha_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .service_def(endpoint_service(unique_name("alpha-service"), alpha_endpoint)),
    );
    let (beta_shutdown, beta_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .service_def(endpoint_service(unique_name("beta-service"), beta_endpoint)),
    );

    let replies_result = request_expected_replies(&client, &full_subject, 2).await;

    let _ = alpha_shutdown.send(());
    let _ = beta_shutdown.send(());

    let alpha_result = alpha_handle.await.context("alpha app task failed")?;
    let beta_result = beta_handle.await.context("beta app task failed")?;

    let replies = replies_result?;
    let actual: BTreeSet<_> = replies.into_iter().collect();
    let expected: BTreeSet<_> = ["alpha".to_string(), "beta".to_string()]
        .into_iter()
        .collect();

    assert_eq!(actual, expected);
    alpha_result?;
    beta_result?;
    Ok(())
}

#[tokio::test]
async fn live_client_drain_triggers_supervised_shutdown() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let unique = unique_name("supervision");
    let group = format!("live.{unique}");
    let full_subject = format!("{group}.status");

    let mut endpoint = LiveSupervisionService::status_endpoint();
    endpoint.group = group;

    let app_handle = tokio::spawn(
        NatsApp::new(client.clone())
            .with_worker_failure_policy(WorkerFailurePolicy::ShutdownApp)
            .service_def(endpoint_service(unique_name("supervision-service"), endpoint))
            .run_until(async {
                pending::<()>().await;
                Ok(())
            }),
    );

    let readiness = request_expected_replies(&client, &full_subject, 1).await;
    client.drain().await.context("failed to drain the shared client")?;
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for supervised shutdown")?
        .context("supervision app task failed")?;

    readiness?;
    let err = app_result.expect_err("client drain should trigger supervised shutdown");
    assert!(err.to_string().contains("worker `endpoint"));
    Ok(())
}

#[tokio::test]
async fn live_jetstream_consumer_processes_messages() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.events", unique_name("consumer-subject"));
    let durable = unique_name("durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live JetStream runtime test: {err}");
            return Ok(());
        }
    };

    let probe = ConsumerProbe::default();
    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable;
    consumer.config.filter_subject = subject.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let publish_result = async {
        jetstream
            .publish(subject.clone(), "hello".into())
            .await
            .context("failed to publish live JetStream test message")?
            .await
            .context("failed to await publish ack")?;

        timeout(Duration::from_secs(5), probe.processed.notified())
            .await
            .context("timed out waiting for live consumer to process message")?;

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let _ = shutdown_tx.send(());
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app shutdown")?
        .context("consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    publish_result?;
    app_result?;
    assert_eq!(probe.hits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn live_endpoint_handlers_observe_shutdown_signal_and_timeout() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let probe = ShutdownProbe::default();
    let unique = unique_name("shutdown-endpoint");
    let group = format!("live.{unique}");
    let status_subject = format!("{group}.status");
    let cleanup_subject = format!("{group}.cleanup");

    let mut status_endpoint = LiveShutdownEndpointService::status_endpoint();
    status_endpoint.group = group.clone();

    let mut cleanup_endpoint = LiveShutdownEndpointService::cleanup_endpoint();
    cleanup_endpoint.group = group.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .with_shutdown_drain_timeout(Duration::from_secs(2))
            .service_def(service_definition(
                unique_name("shutdown-endpoint-service"),
                vec![status_endpoint, cleanup_endpoint],
                Vec::new(),
            )),
    );

    request_expected_replies(&client, &status_subject, 1).await?;

    let request_client = client.clone();
    let request_handle = tokio::spawn(async move {
        let message = timeout(
            Duration::from_secs(5),
            request_client.request(cleanup_subject, "".into()),
        )
        .await
        .context("timed out waiting for cleanup endpoint response")?
        .context("cleanup endpoint request failed")?;

        String::from_utf8(message.payload.to_vec()).context("cleanup reply was not valid UTF-8")
    });

    timeout(Duration::from_secs(5), probe.started.notified())
        .await
        .context("timed out waiting for endpoint handler to start")?;

    let _ = shutdown_tx.send(());

    let reply = request_handle
        .await
        .context("cleanup request task failed")??;
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for endpoint app shutdown")?
        .context("endpoint app task failed")?;

    assert_eq!(reply, "cleaned");
    assert_eq!(
        probe.observation(),
        ShutdownObservation {
            requested: true,
            drain_timeout: Some(Duration::from_secs(2)),
            deadline_present: true,
        }
    );
    app_result?;
    Ok(())
}

#[tokio::test]
async fn live_consumer_handlers_observe_shutdown_signal_and_timeout() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("LIVE_SHUTDOWN_STREAM").to_uppercase();
    let subject = format!("live.{}.shutdown", unique_name("consumer-shutdown-subject"));
    let durable = unique_name("shutdown-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live shutdown consumer test: {err}");
            return Ok(());
        }
    };

    let probe = ShutdownProbe::default();
    let unique = unique_name("shutdown-consumer");
    let group = format!("live.{unique}");
    let status_subject = format!("{group}.status");

    let mut status_endpoint = LiveShutdownConsumerService::status_endpoint();
    status_endpoint.group = group;

    let mut consumer = LiveShutdownConsumerService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable;
    consumer.config.filter_subject = subject.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .with_shutdown_drain_timeout(Duration::from_secs(2))
            .service_def(service_definition(
                unique_name("shutdown-consumer-service"),
                vec![status_endpoint],
                vec![consumer],
            )),
    );

    request_expected_replies(&client, &status_subject, 1).await?;

    jetstream
        .publish(subject.clone(), "hello".into())
        .await
        .context("failed to publish shutdown consumer message")?
        .await
        .context("failed to await shutdown consumer publish ack")?;

    timeout(Duration::from_secs(5), probe.started.notified())
        .await
        .context("timed out waiting for consumer handler to start")?;

    let _ = shutdown_tx.send(());

    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for shutdown consumer app")?
        .context("shutdown consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    assert_eq!(
        probe.observation(),
        ShutdownObservation {
            requested: true,
            drain_timeout: Some(Duration::from_secs(2)),
            deadline_present: true,
        }
    );
    app_result?;
    Ok(())
}

async fn connect_live_nats() -> Option<async_nats::Client> {
    let nats_url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());

    match async_nats::connect(&nats_url).await {
        Ok(client) => Some(client),
        Err(err) => {
            eprintln!("skipping live runtime tests because NATS is unavailable at {nats_url}: {err}");
            None
        }
    }
}

#[tokio::test]
async fn live_consumer_promotes_concurrency_limit_to_max_ack_pending() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("PROMOTE_LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.promote", unique_name("consumer-promote-subject"));
    let durable = unique_name("promote-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live promote test: {err}");
            return Ok(());
        }
    };

    let probe = ConsumerProbe::default();
    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable.clone();
    consumer.config.filter_subject = subject.clone();
    // set server consumer max_ack_pending higher than the app default to exercise promotion
    consumer.config.max_ack_pending = 5;

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .with_default_concurrency_limit(1)
            .state(probe.clone())
            .service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let publish_result = async {
        jetstream
            .publish(subject.clone(), "hello".into())
            .await
            .context("failed to publish live JetStream test message")?
            .await
            .context("failed to await publish ack")?;

        timeout(Duration::from_secs(5), probe.processed.notified())
            .await
            .context("timed out waiting for live consumer to process message")?;

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let _ = shutdown_tx.send(());
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app shutdown")?
        .context("consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    publish_result?;
    app_result?;
    assert_eq!(probe.hits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn live_consumer_configured_concurrency_exceeding_max_ack_pending_errors() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("BAD_LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.bad", unique_name("consumer-bad-subject"));
    let durable = unique_name("bad-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live bad-config test: {err}");
            return Ok(());
        }
    };

    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable.clone();
    consumer.config.filter_subject = subject.clone();
    // set a small server max_ack_pending and an explicit concurrency_limit larger than it
    consumer.config.max_ack_pending = 2;
    consumer.concurrency_limit = Some(5);

    let (_shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone()).service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app")?
        .context("consumer app task failed")?;

    // Should have errored due to invalid configured concurrency limit
    let err = app_result
        .expect_err("app should have errored due to invalid configured concurrency limit");
    let err_str = err.to_string();
    assert!(err_str.contains("invalid configured concurrency limit"), "unexpected error: {err_str}");

    let _ = jetstream.delete_stream(&stream_name).await;
    Ok(())
}

fn spawn_app_until_shutdown(app: NatsApp) -> (oneshot::Sender<()>, JoinHandle<Result<()>>) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        app.run_until(async move {
            let _ = shutdown_rx.await;
            Ok(())
        })
        .await
    });
    (shutdown_tx, handle)
}

fn endpoint_service(name: String, endpoint: EndpointDefinition) -> ServiceDefinition {
    service_definition(name, vec![endpoint], Vec::new())
}

fn consumer_service(name: String, consumer: ConsumerDefinition) -> ServiceDefinition {
    service_definition(name, Vec::new(), vec![consumer])
}

fn service_definition(
    name: String,
    endpoints: Vec<EndpointDefinition>,
    consumers: Vec<ConsumerDefinition>,
) -> ServiceDefinition {
    ServiceDefinition {
        metadata: ServiceMetadata::new(name, "0.1.0", "", None),
        endpoints,
        consumers,
        endpoint_info: Vec::new(),
        consumer_info: Vec::new(),
    }
}

async fn request_expected_replies(
    client: &async_nats::Client,
    subject: &str,
    expected: usize,
) -> Result<Vec<String>> {
    for _ in 0..20 {
        let replies = request_replies_once(client, subject, expected).await?;
        if replies.len() == expected {
            return Ok(replies);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    anyhow::bail!(
        "timed out waiting for {expected} replies on subject `{subject}`"
    )
}

async fn request_replies_once(
    client: &async_nats::Client,
    subject: &str,
    expected: usize,
) -> Result<Vec<String>> {
    let inbox = client.new_inbox();
    let mut replies = client
        .subscribe(inbox.clone())
        .await
        .context("failed to subscribe to reply inbox")?;

    client
        .publish_with_reply(subject.to_string(), inbox, "".into())
        .await
        .with_context(|| format!("failed to publish test request to `{subject}`"))?;
    client.flush().await.context("failed to flush live test request")?;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    let mut payloads = Vec::new();

    while payloads.len() < expected {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match timeout(remaining, replies.next()).await {
            Ok(Some(message)) => payloads.push(
                String::from_utf8(message.payload.to_vec())
                    .context("reply payload was not valid UTF-8")?,
            ),
            Ok(None) | Err(_) => break,
        }
    }

    Ok(payloads)
}

fn unique_name(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}