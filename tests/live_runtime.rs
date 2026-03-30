use std::{
    collections::BTreeSet,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use futures::StreamExt;
use nats_micro::{
    ConsumerDefinition, EndpointDefinition, NatsApp, NatsErrorResponse, Payload, ServiceDefinition,
    ServiceMetadata, State, WorkerFailurePolicy, async_nats, service, service_handlers,
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
    ServiceDefinition {
        metadata: ServiceMetadata::new(name, "0.1.0", "", None),
        endpoints: vec![endpoint],
        consumers: Vec::new(),
        endpoint_info: Vec::new(),
        consumer_info: Vec::new(),
    }
}

fn consumer_service(name: String, consumer: ConsumerDefinition) -> ServiceDefinition {
    ServiceDefinition {
        metadata: ServiceMetadata::new(name, "0.1.0", "", None),
        endpoints: Vec::new(),
        consumers: vec![consumer],
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