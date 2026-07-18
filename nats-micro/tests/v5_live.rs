#![cfg(feature = "live-test")]

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nats_micro::{
    App, Body, ClientError, ClientTransport, ConsumerAction, NatsTransport, Text, message, service,
    service_error,
    testing::{LiveTestApp, TestApp},
};

#[message]
pub struct ParityInput<'a> {
    pub value: &'a str,
}

#[message]
#[derive(Debug)]
pub struct ParityOutput {
    pub value: String,
}

#[service_error]
pub enum ParityError {
    #[error(code = 409, message = "value {value} conflicts")]
    Conflict { value: String },
}

#[service(name = "parity-live", version = "1.0.0")]
impl ParityService {
    #[request("echo")]
    async fn echo(input: ParityInput<'_>) -> ParityOutput {
        ParityOutput {
            value: input.value.to_owned(),
        }
    }

    #[request("lookup.{item_id}")]
    async fn lookup(item_id: &str) -> ParityOutput {
        ParityOutput {
            value: item_id.to_owned(),
        }
    }

    #[request("header")]
    async fn header(#[header("x-parity")] value: &str) -> ParityOutput {
        ParityOutput {
            value: value.to_owned(),
        }
    }

    #[request("optional")]
    async fn optional() -> Option<ParityOutput> {
        None
    }

    #[request("fail")]
    async fn fail() -> std::result::Result<ParityOutput, ParityError> {
        Err(ParityError::Conflict {
            value: "duplicate".to_owned(),
        })
    }
}

async fn assert_parity<T>(client: ParityServiceClient<T>) -> nats_micro::Result<()>
where
    T: ClientTransport,
{
    assert_eq!(
        client.echo(&ParityInput { value: "same" }).await?.value,
        "same"
    );
    assert_eq!(client.lookup("item-42").await?.value, "item-42");
    assert_eq!(
        client
            .header_call()
            .header("x-parity", "header-value")?
            .send()
            .await?
            .value,
        "header-value"
    );
    assert!(client.optional().await?.is_none());
    match client.fail().await.unwrap_err() {
        ClientError::Service { error, response } => {
            assert!(matches!(
                error,
                ParityError::Conflict { ref value } if value == "duplicate"
            ));
            assert_eq!(response.code, 409);
            assert_eq!(response.kind, "CONFLICT");
        }
        other => panic!("expected typed parity error, received {other:?}"),
    }
    Ok(())
}

async fn require_nats_server() -> nats_micro::Result<()> {
    if nats_server::is_server_available() {
        return Ok(());
    }

    let unavailable = LiveTestApp::new().start().await?;
    unavailable.shutdown().await
}

#[nats_micro::live_test]
async fn generated_vectors_match_local_and_live_transports() -> nats_micro::Result<()> {
    let local = TestApp::stateless().serve(ParityService).start();
    assert_parity(local.client::<ParityService>()).await?;

    let live = LiveTestApp::new().serve(ParityService).start().await?;
    assert_parity(live.client::<ParityService>()).await?;

    let discovery = live
        .nats_client()
        .request("$SRV.INFO.parity-live", nats_micro::Bytes::new())
        .await?;
    let info: nats_micro::async_nats::service::Info = serde_json::from_slice(&discovery.payload)?;
    assert_eq!(info.name, "parity-live");
    assert_eq!(info.version, "1.0.0");
    assert!(
        info.endpoints
            .iter()
            .any(|endpoint| endpoint.subject == "parity-live.v1.echo")
    );

    live.shutdown().await
}

pub struct ReplicaState {
    id: &'static str,
}

#[service(
    name = "live-replica",
    version = "1.0.0",
    state = ReplicaState,
    defaults(queue = "live-replicas")
)]
impl ReplicaService {
    #[request("identity")]
    async fn identity(state: &ReplicaState) -> &'static str {
        state.id
    }
}

#[nats_micro::live_test]
async fn replicas_balance_and_reconnect_after_server_restart() -> nats_micro::Result<()> {
    let mut live = LiveTestApp::new()
        .state(ReplicaState { id: "alpha" })
        .serve(ReplicaService)
        .start()
        .await?;
    let second_client = nats_micro::async_nats::connect(live.server_url()).await?;
    let second = App::new(ReplicaState { id: "beta" })
        .with_client(second_client)
        .serve(ReplicaService)
        .start()
        .await?;
    second.ready().await?;

    let replicas = live.client::<ReplicaService>();
    let mut observed = BTreeSet::new();
    for _ in 0..40 {
        observed.insert(replicas.identity().await?);
        if observed.len() == 2 {
            break;
        }
    }
    assert_eq!(
        observed,
        BTreeSet::from(["alpha".to_owned(), "beta".to_owned()])
    );

    live.server_mut().restart();
    let reconnected = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(identity) = replicas
                .identity_call()
                .timeout(Duration::from_millis(250))
                .send()
                .await
            {
                break identity;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    assert!(matches!(reconnected.as_str(), "alpha" | "beta"));

    second.shutdown().await?;
    live.shutdown().await
}

#[nats_micro::live_test]
async fn credentials_permissions_and_lame_duck_are_enforced_live() -> nats_micro::Result<()> {
    require_nats_server().await?;

    let auth_config = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nats-server/configs/auth.conf"
    );
    let auth_server =
        tokio::task::spawn_blocking(move || nats_server::run_server(auth_config)).await?;
    let auth_url = auth_server.client_url();

    let authenticated = nats_micro::async_nats::ConnectOptions::with_user_and_password(
        "app".to_owned(),
        "secret".to_owned(),
    )
    .connect(auth_url.clone())
    .await?;
    let app = App::stateless()
        .with_client(authenticated.clone())
        .serve(ParityService)
        .start()
        .await?;
    app.ready().await?;
    assert_eq!(
        ParityServiceClient::new(NatsTransport::new(authenticated))
            .echo(&ParityInput {
                value: "authenticated"
            })
            .await?
            .value,
        "authenticated"
    );

    nats_micro::async_nats::ConnectOptions::with_user_and_password(
        "app".to_owned(),
        "wrong-password".to_owned(),
    )
    .connect(auth_url.clone())
    .await
    .expect_err("invalid credentials must be rejected");

    let (permission_tx, mut permission_rx) = tokio::sync::mpsc::unbounded_channel();
    let restricted = nats_micro::async_nats::ConnectOptions::with_user_and_password(
        "restricted".to_owned(),
        "secret".to_owned(),
    )
    .event_callback(move |event| {
        let permission_tx = permission_tx.clone();
        async move {
            let _ = permission_tx.send(event);
        }
    })
    .connect(auth_url)
    .await?;
    let _forbidden = restricted.subscribe("denied.subject").await?;
    restricted.flush().await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = permission_rx
                .recv()
                .await
                .expect("permission event channel remains open");
            if let nats_micro::async_nats::Event::ServerError(
                nats_micro::async_nats::ServerError::Other(message),
            ) = event
                && message
                    .to_ascii_lowercase()
                    .contains("permissions violation")
            {
                break;
            }
        }
    })
    .await?;

    app.shutdown().await?;
    drop(restricted);
    drop(auth_server);

    let lame_duck_server = tokio::task::spawn_blocking(nats_server::run_basic_server).await?;
    let lame_duck_url = lame_duck_server.client_url();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let connected = nats_micro::async_nats::ConnectOptions::new()
        .event_callback(move |event| {
            let event_tx = event_tx.clone();
            async move {
                let _ = event_tx.send(event);
            }
        })
        .connect(lame_duck_url)
        .await?;
    nats_server::set_lame_duck_mode(&lame_duck_server);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                event_rx.recv().await,
                Some(nats_micro::async_nats::Event::LameDuckMode)
            ) {
                break;
            }
        }
    })
    .await?;
    drop(connected);
    drop(lame_duck_server);
    Ok(())
}

#[nats_micro::live_test]
async fn generated_service_routes_across_a_real_cluster() -> nats_micro::Result<()> {
    require_nats_server().await?;

    let cluster = tokio::task::spawn_blocking(|| nats_server::run_cluster("")).await?;
    let app_client = nats_micro::async_nats::connect(cluster.servers[0].client_url()).await?;
    let request_client = nats_micro::async_nats::connect(cluster.servers[2].client_url()).await?;
    let app = App::stateless()
        .with_client(app_client)
        .serve(ParityService)
        .start()
        .await?;
    app.ready().await?;

    let generated = ParityServiceClient::new(NatsTransport::new(request_client));
    let value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match generated
                .echo_call(&ParityInput { value: "cluster" })
                .timeout(Duration::from_millis(200))
                .send()
                .await
            {
                Ok(response) => break response.value,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await?;
    assert_eq!(value, "cluster");

    app.shutdown().await?;
    drop(cluster);
    Ok(())
}

pub struct ConsumerState {
    delayed: Arc<AtomicUsize>,
    terminal: Arc<AtomicUsize>,
    progress: Arc<AtomicUsize>,
    delayed_ready: Arc<tokio::sync::Notify>,
    terminal_ready: Arc<tokio::sync::Notify>,
    progress_ready: Arc<tokio::sync::Notify>,
}

#[service(
    name = "live-consumers",
    version = "1.0.0",
    state = ConsumerState
)]
impl LiveConsumerService {
    #[consumer(
        stream = "LIVE_EVENTS",
        durable = "live-delayed",
        filter = "live.events.delayed",
        ack_wait = "1s",
        max_deliver = 3,
        backoff = ["50ms", "100ms"]
    )]
    async fn delayed(state: &ConsumerState, body: Body<'_>) -> ConsumerAction {
        let _ = body;
        let attempt = state.delayed.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt == 1 {
            ConsumerAction::NackAfter(Duration::from_millis(50))
        } else {
            state.delayed_ready.notify_one();
            ConsumerAction::Ack
        }
    }

    #[consumer(
        stream = "LIVE_EVENTS",
        durable = "live-terminal",
        filter = "live.events.terminal",
        ack_wait = "1s"
    )]
    async fn terminal(state: &ConsumerState, text: Text<'_>) -> ConsumerAction {
        let _ = text;
        state.terminal.fetch_add(1, Ordering::SeqCst);
        state.terminal_ready.notify_one();
        ConsumerAction::Term
    }

    #[consumer(
        stream = "LIVE_EVENTS",
        durable = "live-progress",
        filter = "live.events.progress",
        ack_wait = "300ms",
        max_deliver = 3
    )]
    async fn progress(state: &ConsumerState, body: Body<'_>) -> ConsumerAction {
        let _ = body;
        state.progress.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(700)).await;
        state.progress_ready.notify_one();
        ConsumerAction::Ack
    }
}

#[nats_micro::live_test]
async fn real_jetstream_configuration_redelivery_term_and_progress_ack() -> nats_micro::Result<()> {
    let delayed = Arc::new(AtomicUsize::new(0));
    let terminal = Arc::new(AtomicUsize::new(0));
    let progress = Arc::new(AtomicUsize::new(0));
    let delayed_ready = Arc::new(tokio::sync::Notify::new());
    let terminal_ready = Arc::new(tokio::sync::Notify::new());
    let progress_ready = Arc::new(tokio::sync::Notify::new());
    let live = LiveTestApp::new()
        .jetstream()
        .state(ConsumerState {
            delayed: Arc::clone(&delayed),
            terminal: Arc::clone(&terminal),
            progress: Arc::clone(&progress),
            delayed_ready: Arc::clone(&delayed_ready),
            terminal_ready: Arc::clone(&terminal_ready),
            progress_ready: Arc::clone(&progress_ready),
        })
        .serve(LiveConsumerService)
        .start()
        .await?;

    live.nats_client()
        .publish(
            "live.events.delayed",
            nats_micro::Bytes::from_static(b"delayed"),
        )
        .await?;
    live.nats_client()
        .publish(
            "live.events.terminal",
            nats_micro::Bytes::from_static(b"terminal"),
        )
        .await?;
    live.nats_client()
        .publish(
            "live.events.progress",
            nats_micro::Bytes::from_static(b"progress"),
        )
        .await?;
    live.nats_client().flush().await?;

    tokio::time::timeout(Duration::from_secs(3), delayed_ready.notified()).await?;
    tokio::time::timeout(Duration::from_secs(3), terminal_ready.notified()).await?;
    tokio::time::timeout(Duration::from_secs(3), progress_ready.notified()).await?;
    assert_eq!(delayed.load(Ordering::SeqCst), 2);
    assert_eq!(terminal.load(Ordering::SeqCst), 1);
    assert_eq!(progress.load(Ordering::SeqCst), 1);

    let jetstream = nats_micro::async_nats::jetstream::new(live.nats_client().clone());
    let stream = jetstream.get_stream("LIVE_EVENTS").await?;
    let info = stream.consumer_info("live-delayed").await?;
    assert_eq!(info.config.max_deliver, 3);
    assert_eq!(info.config.ack_wait, Duration::from_millis(50));
    assert_eq!(
        info.config.backoff,
        vec![Duration::from_millis(50), Duration::from_millis(100)]
    );

    live.shutdown().await
}
