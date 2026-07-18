#![cfg(feature = "protobuf")]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::Duration,
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use nats_micro::{
    App, Body, Bytes, ClientError, ClientRequest, ClientResponse, ClientSubject, ClientTransport,
    NatsTransport, Proto, Service, Text, message, service, service_error,
};

#[message]
pub struct EchoInput<'a> {
    pub value: &'a str,
}

#[message]
#[derive(Debug)]
pub struct EchoOutput {
    pub value: String,
}

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
pub struct ProtoInput {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
pub struct ProtoOutput {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[service_error]
pub enum ClientTestError {
    #[error(code = 409, message = "value {value} conflicts")]
    Conflict { value: String },
}

#[service(name = "phase3-client", version = "1.0.0")]
impl ClientRoundtripService {
    #[request("echo")]
    async fn echo(input: EchoInput<'_>) -> EchoOutput {
        EchoOutput {
            value: input.value.to_owned(),
        }
    }

    #[request("get.{item_id}")]
    async fn get(item_id: &str) -> EchoOutput {
        EchoOutput {
            value: item_id.to_owned(),
        }
    }

    #[request("tenant")]
    async fn tenant(#[header("x-tenant-id")] tenant: &str) -> EchoOutput {
        EchoOutput {
            value: tenant.to_owned(),
        }
    }

    #[request("fail")]
    async fn fail() -> Result<EchoOutput, ClientTestError> {
        Err(ClientTestError::Conflict {
            value: "duplicate".to_owned(),
        })
    }

    #[request("optional")]
    async fn optional() -> Option<EchoOutput> {
        None
    }

    #[request("raw")]
    async fn raw(body: Body<'_>) -> Bytes {
        Bytes::copy_from_slice(body.0)
    }

    #[request("text")]
    async fn text(input: Text<'_>) -> &'static str {
        let _ = input;
        "text-response"
    }

    #[request("proto")]
    async fn proto(input: Proto<ProtoInput>) -> Proto<ProtoOutput> {
        Proto(ProtoOutput {
            value: input.0.value,
        })
    }

    #[request("panic-control")]
    async fn panic_control(input: Text<'_>) -> &'static str {
        assert_ne!(input.0, "panic", "intentional handler panic");
        "worker-alive"
    }

    #[publish("events.{item_id}")]
    async fn event(item_id: &str, input: EchoInput<'_>) {
        let _ = (item_id, input);
    }
}

#[service(name = "phase3-client", version = "1.0.0")]
impl DuplicateClientService {
    #[request("echo")]
    async fn duplicate() {}
}

#[service(name = "phase3-second", version = "1.0.0")]
impl SecondService {
    #[request("ready")]
    async fn ready() -> &'static str {
        "second-ready"
    }
}

struct WorkerState {
    subscription_seen: std::sync::Arc<tokio::sync::Notify>,
    consumer_seen: std::sync::Arc<tokio::sync::Notify>,
}

#[service(
    name = "phase3-workers",
    version = "1.0.0",
    state = WorkerState
)]
impl WorkerService {
    #[subscribe("core.events", concurrency = 2)]
    async fn observe(state: &WorkerState, body: Body<'_>) {
        let _ = body;
        state.subscription_seen.notify_one();
    }

    #[consumer(
        stream = "PHASE3_WORKERS",
        durable = "phase3-worker",
        filter = "phase3.workers.events",
        concurrency = 2,
        ack_wait = "1s"
    )]
    async fn consume(state: &WorkerState, body: Body<'_>) -> nats_micro::ConsumerAction {
        let _ = body;
        state.consumer_seen.notify_one();
        nats_micro::ConsumerAction::Ack
    }
}

#[tokio::test]
async fn fluent_app_rejects_duplicates_before_connecting() {
    let result = App::stateless()
        .serve(ClientRoundtripService)
        .serve(DuplicateClientService)
        .start()
        .await;
    let Err(error) = result else {
        panic!("duplicate subjects should fail startup");
    };
    assert!(error.to_string().contains("duplicate subject"));
    assert!(error.to_string().contains("phase3-client.v1.echo"));
}

#[tokio::test]
async fn typed_app_and_generated_client_cover_the_wire_contract() -> Result<()> {
    let server = nats_server::run_basic_server();
    let client = nats_micro::async_nats::connect(server.client_url()).await?;
    let startup_completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let startup_observer = std::sync::Arc::clone(&startup_completed);
    let shutdown_completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_observer = std::sync::Arc::clone(&shutdown_completed);
    let running = App::stateless()
        .with_client(client.clone())
        .serve(ClientRoundtripService)
        .serve(SecondService)
        .startup_hook(move |_| async move {
            startup_observer.store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        })
        .shutdown_hook(move || {
            let shutdown_observer = std::sync::Arc::clone(&shutdown_observer);
            async move {
                shutdown_observer.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            }
        })
        .start()
        .await?;
    running.ready().await?;
    assert!(startup_completed.load(std::sync::atomic::Ordering::Acquire));

    let generated = ClientRoundtripService::client(NatsTransport::new(client.clone()));
    let second = SecondService::client(NatsTransport::new(client.clone()));
    assert_eq!(second.ready().await?, "second-ready");
    let echoed = generated.echo(&EchoInput { value: "hello" }).await?;
    assert_eq!(echoed.value, "hello");

    let dynamic = generated.get("item-42").await?;
    assert_eq!(dynamic.value, "item-42");

    let tenant = generated
        .tenant_call()
        .header("x-tenant-id", "tenant-a")?
        .timeout(Duration::from_secs(2))
        .send()
        .await?;
    assert_eq!(tenant.value, "tenant-a");

    let error = generated.fail().await.unwrap_err();
    match error {
        ClientError::Service { error, response } => {
            assert!(matches!(
                error,
                ClientTestError::Conflict { ref value } if value == "duplicate"
            ));
            assert_eq!(response.code, 409);
            assert_eq!(response.kind, "CONFLICT");
        }
        other => panic!("expected typed service error, received {other:?}"),
    }

    assert!(generated.optional().await?.is_none());
    assert_eq!(
        generated.raw(b"raw-request").await?,
        Bytes::from_static(b"raw-request")
    );
    assert_eq!(generated.text("ignored").await?, "text-response");
    assert_eq!(
        generated
            .proto(&ProtoInput {
                value: "protobuf".to_owned(),
            })
            .await?
            .value,
        "protobuf"
    );

    assert!(
        generated
            .panic_control_call("panic")
            .timeout(Duration::from_millis(100))
            .send()
            .await
            .is_err()
    );
    assert_eq!(generated.panic_control("continue").await?, "worker-alive");

    let mut events = client.subscribe("phase3-client.v1.events.item-42").await?;
    generated
        .event("item-42", &EchoInput { value: "published" })
        .await?;
    let event = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .context("generated publish timed out")?
        .context("generated publish subscription ended")?;
    let payload: EchoInput<'_> = serde_json::from_slice(&event.payload)?;
    assert_eq!(payload.value, "published");

    running.shutdown().await?;
    assert!(shutdown_completed.load(std::sync::atomic::Ordering::Acquire));
    Ok(())
}

#[tokio::test]
async fn empty_app_waits_for_shutdown_and_failed_startup_cleans_up() -> Result<()> {
    let server = nats_server::run_basic_server();
    let empty_client = nats_micro::async_nats::connect(server.client_url()).await?;
    let empty_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let empty_shutdown_observer = std::sync::Arc::clone(&empty_shutdown);
    let empty = App::stateless()
        .with_client(empty_client)
        .shutdown_hook(move || {
            let empty_shutdown_observer = std::sync::Arc::clone(&empty_shutdown_observer);
            async move {
                empty_shutdown_observer.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            }
        })
        .start()
        .await?;
    empty.ready().await?;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(!empty_shutdown.load(std::sync::atomic::Ordering::Acquire));
    empty.shutdown().await?;
    assert!(empty_shutdown.load(std::sync::atomic::Ordering::Acquire));

    let failed_client = nats_micro::async_nats::connect(server.client_url()).await?;
    let failed_cleanup = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let failed_cleanup_observer = std::sync::Arc::clone(&failed_cleanup);
    let failed = App::stateless()
        .with_client(failed_client)
        .startup_hook(|_| async { anyhow::bail!("intentional startup failure") })
        .shutdown_hook(move || {
            let failed_cleanup_observer = std::sync::Arc::clone(&failed_cleanup_observer);
            async move {
                failed_cleanup_observer.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            }
        })
        .start()
        .await;
    let Err(failed) = failed else {
        panic!("startup hook should fail");
    };
    assert!(failed.to_string().contains("intentional startup failure"));
    assert!(failed_cleanup.load(std::sync::atomic::Ordering::Acquire));
    Ok(())
}

#[tokio::test]
async fn failed_worker_restarts_with_bounded_backoff() -> Result<()> {
    let server = nats_server::run_basic_server();
    let client = nats_micro::async_nats::connect(server.client_url()).await?;
    let mut config = nats_micro::AppConfig::for_profile(nats_micro::Profile::Test);
    config.handler_panic = nats_micro::HandlerPanicPolicy::FailWorker;
    config.worker_failure = nats_micro::WorkerFailurePolicy::Restart {
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(20),
        max_restarts: 2,
        window: Duration::from_secs(1),
    };
    let running = App::stateless()
        .with_client(client.clone())
        .config(config)
        .serve(ClientRoundtripService)
        .start()
        .await?;
    running.ready().await?;

    let generated = ClientRoundtripService::client(NatsTransport::new(client));
    assert!(
        generated
            .panic_control_call("panic")
            .timeout(Duration::from_millis(100))
            .send()
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        generated.panic_control("after-restart").await?,
        "worker-alive"
    );

    running.shutdown().await
}

#[tokio::test]
async fn readiness_waits_for_subscriptions_and_consumers() -> Result<()> {
    let server = nats_server::run_server_with_jetstream();
    let client = nats_micro::async_nats::connect(server.client_url()).await?;
    let jetstream = nats_micro::async_nats::jetstream::new(client.clone());
    jetstream
        .create_stream(nats_micro::async_nats::jetstream::stream::Config {
            name: "PHASE3_WORKERS".to_owned(),
            subjects: vec!["phase3.workers.events".to_owned()],
            ..Default::default()
        })
        .await?;

    let subscription_seen = std::sync::Arc::new(tokio::sync::Notify::new());
    let consumer_seen = std::sync::Arc::new(tokio::sync::Notify::new());
    let running = App::new(WorkerState {
        subscription_seen: std::sync::Arc::clone(&subscription_seen),
        consumer_seen: std::sync::Arc::clone(&consumer_seen),
    })
    .with_client(client.clone())
    .serve(WorkerService)
    .start()
    .await?;
    running.ready().await?;

    client
        .publish("phase3-workers.v1.core.events", Bytes::from_static(b"core"))
        .await?;
    client
        .publish("phase3.workers.events", Bytes::from_static(b"durable"))
        .await?;
    client.flush().await?;
    tokio::time::timeout(Duration::from_secs(2), subscription_seen.notified())
        .await
        .context("core subscription did not become active before readiness")?;
    tokio::time::timeout(Duration::from_secs(2), consumer_seen.notified())
        .await
        .context("JetStream consumer did not become active before readiness")?;

    running.shutdown().await
}

#[tokio::test]
async fn partial_service_startup_failure_stops_registered_workers() -> Result<()> {
    let server = nats_server::run_server_with_jetstream();
    let app_client = nats_micro::async_nats::connect(server.client_url()).await?;
    let publisher = nats_micro::async_nats::connect(server.client_url()).await?;
    let subscription_seen = std::sync::Arc::new(tokio::sync::Notify::new());
    let result = App::new(WorkerState {
        subscription_seen: std::sync::Arc::clone(&subscription_seen),
        consumer_seen: std::sync::Arc::new(tokio::sync::Notify::new()),
    })
    .with_client(app_client)
    .serve(WorkerService)
    .start()
    .await;
    assert!(result.is_err(), "missing stream should fail startup");

    publisher
        .publish(
            "phase3-workers.v1.core.events",
            Bytes::from_static(b"after-failure"),
        )
        .await?;
    publisher.flush().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), subscription_seen.notified())
            .await
            .is_err(),
        "subscription worker remained active after failed startup"
    );
    Ok(())
}

struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNTING.with(|counting| {
            if counting.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_allocations(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|allocations| allocations.set(0));
    COUNTING.with(|counting| counting.set(true));
    f();
    COUNTING.with(|counting| counting.set(false));
    ALLOCATIONS.with(Cell::get)
}

#[derive(Clone)]
struct NeverTransport;

impl ClientTransport for NeverTransport {
    async fn request<'a>(
        &'a self,
        _request: ClientRequest<'a>,
    ) -> Result<ClientResponse, nats_micro::TransportError> {
        unreachable!("allocation test does not send")
    }

    async fn publish<'a>(
        &'a self,
        _subject: ClientSubject<'a>,
        _payload: Bytes,
        _headers: Option<nats_micro::NatsHeaderMap>,
    ) -> Result<(), nats_micro::TransportError> {
        unreachable!("allocation test does not send")
    }
}

#[test]
fn generated_subjects_have_the_required_allocation_shape() {
    let client = ClientRoundtripService::client(NeverTransport);

    let static_allocations = count_allocations(|| {
        let call = client.optional_call();
        std::hint::black_box(call);
    });
    assert_eq!(static_allocations, 0);

    let dynamic_allocations = count_allocations(|| {
        let call = client.get_call("item-42");
        std::hint::black_box(call);
    });
    assert_eq!(dynamic_allocations, 1);
}
