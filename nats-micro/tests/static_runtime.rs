use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use nats_micro::async_nats::service::ServiceExt;
use nats_micro::{
    AuthPolicy, Bytes, Codec, ConsumerAction, ConsumerHandler, ConsumerSpec, DispatchResult,
    ErrorReply, FrameworkError, OperationKind, OperationSpec, Request, RequestEndpoint, Response,
    ShutdownState, async_nats,
    runtime::{run_consumer, run_request_endpoint},
};
use tokio::sync::{Notify, watch};

struct StaticEndpoint;

impl RequestEndpoint<AtomicUsize> for StaticEndpoint {
    const SPEC: OperationSpec = OperationSpec {
        rust_name: "echo",
        kind: OperationKind::Request,
        subject: "phase1.static.echo",
        subject_template: "phase1.static.echo",
        queue_group: None,
        request_codec: Codec::Raw,
        response_codec: Codec::Raw,
        request_type: Some("&[u8]"),
        response_type: Some("&'static [u8]"),
        error_type: Some("ErrorReply"),
        auth: AuthPolicy::None,
        concurrency: 4,
        params: &[],
    };

    async fn call<'a>(state: &'a AtomicUsize, request: Request<'a>) -> DispatchResult {
        state.fetch_add(1, Ordering::Relaxed);
        if request.body() == b"ping" {
            Ok(Response::bytes(Bytes::from_static(b"pong")))
        } else {
            Err(ErrorReply::framework(
                FrameworkError::BadJson,
                "expected ping",
                request.request_id().existing(),
            ))
        }
    }
}

struct ConsumerState {
    calls: AtomicUsize,
    handled: Notify,
}

struct StaticConsumer;

impl ConsumerHandler<ConsumerState> for StaticConsumer {
    const SPEC: ConsumerSpec = ConsumerSpec {
        rust_name: "project_event",
        stream: "PHASE1_STATIC",
        durable: "phase1-static-consumer",
        filter_subject: "phase1.events",
        concurrency: 2,
        ack_wait_ms: 300,
        max_deliver: 3,
        backoff_ms: &[],
    };

    async fn call<'a>(
        state: &'a ConsumerState,
        request: Request<'a>,
    ) -> Result<ConsumerAction, ErrorReply> {
        assert_eq!(request.subject(), Self::SPEC.filter_subject);
        assert_eq!(request.body(), b"event");
        state.calls.fetch_add(1, Ordering::Relaxed);
        state.handled.notify_one();
        Ok(ConsumerAction::Ack)
    }
}

#[tokio::test]
async fn handwritten_static_endpoint_round_trips_over_live_nats() -> Result<()> {
    let server = nats_server::run_basic_server();
    let client = async_nats::connect(server.client_url()).await?;
    let service = client
        .service_builder()
        .description("phase 1 static runtime")
        .start("phase1-static", "2.0.0")
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let endpoint = service
        .endpoint_builder()
        .name(StaticEndpoint::SPEC.rust_name)
        .add(StaticEndpoint::SPEC.subject)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let state = Arc::new(AtomicUsize::new(0));
    let (shutdown_tx, shutdown_rx) = watch::channel(ShutdownState::running(None));
    let worker = tokio::spawn(run_request_endpoint::<AtomicUsize, StaticEndpoint>(
        Arc::clone(&state),
        endpoint,
        StaticEndpoint::SPEC.concurrency,
        shutdown_rx,
    ));

    let success = client
        .request(StaticEndpoint::SPEC.subject, Bytes::from_static(b"ping"))
        .await?;
    assert_eq!(success.payload, Bytes::from_static(b"pong"));
    assert!(
        success
            .headers
            .as_ref()
            .is_none_or(async_nats::HeaderMap::is_empty),
        "success must not allocate protocol marker headers"
    );

    let mut headers = async_nats::HeaderMap::new();
    headers.insert("x-request-id", "phase1-request");
    let failure = client
        .request_with_headers(
            StaticEndpoint::SPEC.subject,
            headers,
            Bytes::from_static(b"bad"),
        )
        .await?;
    let failure_headers = failure.headers.context("missing service error headers")?;
    assert_eq!(
        failure_headers
            .get("Nats-Service-Error-Code")
            .map(async_nats::HeaderValue::as_str),
        Some("400")
    );
    assert_eq!(
        failure_headers
            .get("Nats-Micro-Error-Kind")
            .map(async_nats::HeaderValue::as_str),
        Some("BAD_JSON")
    );
    assert_eq!(
        failure_headers
            .get("x-request-id")
            .map(async_nats::HeaderValue::as_str),
        Some("phase1-request")
    );
    let failure_json: serde_json::Value = serde_json::from_slice(&failure.payload)?;
    assert_eq!(failure_json["kind"], "BAD_JSON");
    assert_eq!(failure_json["request_id"], "phase1-request");
    assert_eq!(state.load(Ordering::Relaxed), 2);

    shutdown_tx.send(ShutdownState::requested(None))?;
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .context("static endpoint did not drain")??
        .context("static endpoint worker failed")?;
    drop(service);
    Ok(())
}

#[tokio::test]
async fn handwritten_static_consumer_round_trips_over_live_jetstream() -> Result<()> {
    let server = nats_server::run_server_with_jetstream();
    let client = async_nats::connect(server.client_url()).await?;
    let jetstream = async_nats::jetstream::new(client.clone());
    let stream = jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: StaticConsumer::SPEC.stream.to_owned(),
            subjects: vec![StaticConsumer::SPEC.filter_subject.to_owned()],
            ..Default::default()
        })
        .await?;
    let consumer = stream
        .get_or_create_consumer(
            StaticConsumer::SPEC.durable,
            async_nats::jetstream::consumer::push::Config {
                durable_name: Some(StaticConsumer::SPEC.durable.to_owned()),
                deliver_subject: client.new_inbox(),
                filter_subject: StaticConsumer::SPEC.filter_subject.to_owned(),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_millis(StaticConsumer::SPEC.ack_wait_ms),
                max_deliver: StaticConsumer::SPEC.max_deliver,
                ..Default::default()
            },
        )
        .await?;
    let messages = consumer.messages().await?;
    let state = Arc::new(ConsumerState {
        calls: AtomicUsize::new(0),
        handled: Notify::new(),
    });
    let (shutdown_tx, shutdown_rx) = watch::channel(ShutdownState::running(None));
    let worker = tokio::spawn(run_consumer::<ConsumerState, StaticConsumer>(
        Arc::clone(&state),
        messages,
        StaticConsumer::SPEC.concurrency,
        shutdown_rx,
    ));

    client
        .publish(
            StaticConsumer::SPEC.filter_subject,
            Bytes::from_static(b"event"),
        )
        .await?;
    client.flush().await?;
    tokio::time::timeout(Duration::from_secs(5), state.handled.notified())
        .await
        .context("static consumer did not receive the event")?;
    assert_eq!(state.calls.load(Ordering::Relaxed), 1);

    shutdown_tx.send(ShutdownState::requested(None))?;
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .context("static consumer did not drain")??
        .context("static consumer worker failed")?;
    Ok(())
}
