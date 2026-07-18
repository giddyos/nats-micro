#![cfg(feature = "test-util")]
#![cfg(feature = "protobuf")]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::Duration,
};

use nats_micro::{
    Auth, AuthError, Body, Bytes, ClientError, ClientRequest, ClientSubject, ClientTransport,
    FromRequestMeta, Proto, RequestMeta, Response, Text, TransportError, message, service,
    service_error,
    testing::{LocalSubject, TestApp},
};

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

#[test]
fn local_subject_conversion_has_the_required_allocation_shape() {
    let static_allocations = count_allocations(|| {
        std::hint::black_box(LocalSubject::from(ClientSubject::Static(
            "local-wire.v1.unit",
        )));
    });
    assert_eq!(static_allocations, 0);

    let owned = "local-wire.v1.lookup.item-42".to_owned();
    let owned_allocations = count_allocations(|| {
        std::hint::black_box(LocalSubject::from(ClientSubject::Owned(owned)));
    });
    assert_eq!(owned_allocations, 0);

    let borrowed = "local-wire.v1.lookup.item-42";
    let borrowed_allocations = count_allocations(|| {
        std::hint::black_box(LocalSubject::from(ClientSubject::Borrowed(borrowed)));
    });
    assert_eq!(borrowed_allocations, 1);
}

#[message]
pub struct JsonInput<'a> {
    pub value: &'a str,
}

#[message]
#[derive(Debug)]
pub struct JsonOutput {
    pub value: String,
}

#[message]
pub struct JsonEvent {
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

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
pub struct ProtoEvent {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[service_error]
pub enum LocalError {
    #[error(code = 409, message = "value {value} conflicts")]
    Conflict { value: String },

    #[internal]
    Internal(#[from] std::io::Error),
}

pub struct Claims {
    subject: String,
}

impl FromRequestMeta for Claims {
    async fn from_request_meta(meta: RequestMeta<'_>) -> Result<Self, AuthError> {
        let token = meta
            .headers
            .get("authorization")
            .ok_or(AuthError::MissingCredentials)?;
        if token != "Bearer local-test" {
            return Err(AuthError::InvalidCredentials);
        }
        Ok(Self {
            subject: "local-user".to_owned(),
        })
    }
}

#[service(name = "local-wire", version = "1.0.0")]
impl LocalWireService {
    #[request("json")]
    async fn json(input: JsonInput<'_>) -> JsonOutput {
        JsonOutput {
            value: input.value.to_owned(),
        }
    }

    #[request("lookup.{item_id}")]
    async fn lookup(item_id: &str) -> JsonOutput {
        JsonOutput {
            value: item_id.to_owned(),
        }
    }

    #[request("proto")]
    async fn proto(input: Proto<ProtoInput>) -> Proto<ProtoOutput> {
        Proto(ProtoOutput {
            value: input.0.value,
        })
    }

    #[request("raw")]
    async fn raw(body: Body<'_>) -> Bytes {
        Bytes::copy_from_slice(body.0)
    }

    #[request("text")]
    async fn text(text: Text<'_>) -> &'static str {
        let _ = text;
        "text-response"
    }

    #[request("unit")]
    async fn unit() {}

    #[request("optional")]
    async fn optional() -> Option<JsonOutput> {
        None
    }

    #[request("headers")]
    async fn headers(
        #[header("x-required")] required: &str,
        #[header("x-optional")] optional: Option<&str>,
    ) -> JsonOutput {
        JsonOutput {
            value: format!("{required}:{}", optional.unwrap_or("none")),
        }
    }

    #[request("auth", auth = required)]
    async fn auth(auth: Auth<Claims>) -> JsonOutput {
        JsonOutput {
            value: auth.subject.clone(),
        }
    }

    #[request("typed-error")]
    async fn typed_error() -> std::result::Result<JsonOutput, LocalError> {
        Err(LocalError::Conflict {
            value: "duplicate".to_owned(),
        })
    }

    #[request("internal-error")]
    async fn internal_error() -> std::result::Result<JsonOutput, LocalError> {
        Err(LocalError::Internal(std::io::Error::other(
            "secret database details",
        )))
    }

    #[request("response-headers")]
    async fn response_headers() -> Response {
        let mut headers = nats_micro::NatsHeaderMap::new();
        headers.insert("x-local-response", "present");
        Response::with_headers(Bytes::from_static(b"response-body"), headers)
    }

    #[request("slow")]
    async fn slow() {
        tokio::time::sleep(Duration::from_mins(1)).await;
    }

    #[publish("json-events.{event_id}")]
    async fn json_event(event_id: &str, event: JsonEvent) {
        let _ = (event_id, event);
    }

    #[publish("proto-events")]
    async fn proto_event(event: Proto<ProtoEvent>) {
        let _ = event;
    }
}

pub struct LocalState {
    prefix: &'static str,
}

#[service(
    name = "local-stateful",
    version = "1.0.0",
    state = LocalState
)]
impl LocalStatefulService {
    #[request("lookup.{item_id}")]
    async fn lookup(state: &LocalState, item_id: &str) -> JsonOutput {
        JsonOutput {
            value: format!("{}:{item_id}", state.prefix),
        }
    }
}

#[nats_micro::test]
async fn stateful_test_app_routes_through_generated_dispatch() -> nats_micro::Result<()> {
    let app = TestApp::new(LocalState { prefix: "state" })
        .serve(LocalStatefulService)
        .start();

    assert_eq!(
        app.client::<LocalStatefulService>()
            .lookup("item-7")
            .await?
            .value,
        "state:item-7"
    );
    Ok(())
}

#[nats_micro::test]
async fn generated_client_covers_the_complete_local_wire_path() -> nats_micro::Result<()> {
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread
    );
    let app = TestApp::stateless().serve(LocalWireService).start();
    let client = app.client::<LocalWireService>();

    let json = client.json(&JsonInput { value: "borrowed" }).await?;
    assert_eq!(json.value, "borrowed");
    assert_eq!(client.lookup("item-42").await?.value, "item-42");
    assert_eq!(
        client
            .proto(&ProtoInput {
                value: "protobuf".to_owned(),
            })
            .await?
            .value,
        "protobuf"
    );
    assert_eq!(
        client.raw(b"raw-bytes").await?,
        Bytes::from_static(b"raw-bytes")
    );
    assert_eq!(client.text("request-text").await?, "text-response");
    client.unit().await?;
    assert!(client.optional().await?.is_none());

    let headers = client
        .headers_call()
        .header("x-required", "required")?
        .header("x-optional", "optional")?
        .send()
        .await?;
    assert_eq!(headers.value, "required:optional");
    let optional_header_absent = client
        .headers_call()
        .header("x-required", "required")?
        .send()
        .await?;
    assert_eq!(optional_header_absent.value, "required:none");

    let auth = client
        .auth_call()
        .header("authorization", "Bearer local-test")?
        .send()
        .await?;
    assert_eq!(auth.value, "local-user");

    let response = client.response_headers().await?;
    assert_eq!(response.payload, Bytes::from_static(b"response-body"));
    assert_eq!(
        response
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-local-response"))
            .map(nats_micro::async_nats::HeaderValue::as_str),
        Some("present")
    );
    Ok(())
}

#[nats_micro::test]
async fn local_errors_match_native_service_error_protocol() -> nats_micro::Result<()> {
    let app = TestApp::stateless().serve(LocalWireService).start();
    let client = app.client::<LocalWireService>();

    match client.typed_error().await.unwrap_err() {
        ClientError::Service { error, response } => {
            assert!(matches!(
                error,
                LocalError::Conflict { ref value } if value == "duplicate"
            ));
            assert_eq!(response.code, 409);
            assert_eq!(response.kind, "CONFLICT");
        }
        other => panic!("expected typed local error, received {other:?}"),
    }

    let internal = client.internal_error().await.unwrap_err();
    let response = internal
        .as_nats_error_response()
        .expect("internal service response");
    assert_eq!(response.code, 500);
    assert_eq!(response.kind, "INTERNAL_ERROR");
    assert!(!response.message.contains("secret database details"));

    let missing_header = client.headers().await.unwrap_err();
    assert_eq!(
        missing_header
            .as_nats_error_response()
            .expect("missing header response")
            .kind,
        "INVALID_HEADER"
    );
    let missing_auth = client.auth().await.unwrap_err();
    assert_eq!(
        missing_auth
            .as_nats_error_response()
            .expect("missing auth response")
            .kind,
        "UNAUTHORIZED"
    );
    Ok(())
}

#[nats_micro::test]
async fn publishes_are_captured_and_typed_without_a_server() -> nats_micro::Result<()> {
    let app = TestApp::stateless().serve(LocalWireService).start();
    let client = app.client::<LocalWireService>();

    client
        .json_event_call(
            "event-7",
            &JsonEvent {
                value: "json-event".to_owned(),
            },
        )
        .header("x-event-source", "local")?
        .send()
        .await?;
    client
        .proto_event(&ProtoEvent {
            value: "proto-event".to_owned(),
        })
        .await?;

    assert_eq!(app.events().all().len(), 2);
    let json: JsonEvent = app
        .events()
        .subject("local-wire.v1.json-events.event-7")
        .single_json()?;
    assert_eq!(json.value, "json-event");
    let headers = app
        .events()
        .subject("local-wire.v1.json-events.event-7")
        .headers()?
        .expect("event headers");
    assert_eq!(
        headers
            .get("x-event-source")
            .map(nats_micro::async_nats::HeaderValue::as_str),
        Some("local")
    );
    let proto: ProtoEvent = app
        .events()
        .subject("local-wire.v1.proto-events")
        .single_proto()?;
    assert_eq!(proto.value, "proto-event");

    app.events()
        .subject("local-wire.v1.json-events.event-7")
        .clear();
    app.events()
        .subject("local-wire.v1.json-events.event-7")
        .assert_count(0);
    app.events()
        .subject("local-wire.v1.proto-events")
        .assert_count(1);
    Ok(())
}

#[nats_micro::test]
async fn unmatched_routes_and_faults_are_deterministic() -> nats_micro::Result<()> {
    let app = TestApp::stateless().serve(LocalWireService).start();
    let client = app.client::<LocalWireService>();
    let transport = app.transport();

    let unmatched = transport
        .request(ClientRequest {
            subject: ClientSubject::Static("missing.v1.route"),
            payload: Bytes::new(),
            headers: None,
            timeout: None,
        })
        .await;
    assert!(matches!(unmatched, Err(TransportError::NoResponders)));

    app.faults().timeout_next("local-wire.v1.unit");
    let timeout = client.unit().await.unwrap_err();
    assert_eq!(
        timeout
            .as_nats_error_response()
            .expect("timeout response")
            .kind,
        "TIMEOUT"
    );

    app.faults().no_responders_for("local-wire.v1.unit");
    let no_responders = client.unit().await.unwrap_err();
    assert_eq!(
        no_responders
            .as_nats_error_response()
            .expect("no responders response")
            .kind,
        "NO_RESPONDERS"
    );

    app.faults().fail_next_publish(
        "local-wire.v1.json-events.event-9",
        TransportError::Disconnected,
    );
    let failed_publish = client
        .json_event(
            "event-9",
            &JsonEvent {
                value: "not-recorded".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        failed_publish
            .as_nats_error_response()
            .expect("publish response")
            .kind,
        "DISCONNECTED"
    );
    app.events()
        .subject("local-wire.v1.json-events.event-9")
        .assert_count(0);
    assert_eq!(app.faults().pending(), 0);
    Ok(())
}

#[nats_micro::test(start_paused = true)]
async fn local_transport_honors_builder_timeouts_with_virtual_time() {
    let app = TestApp::stateless().serve(LocalWireService).start();
    let client = app.client::<LocalWireService>();

    let error = client
        .slow_call()
        .timeout(Duration::from_secs(1))
        .send()
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_nats_error_response()
            .expect("timeout response")
            .kind,
        "TIMEOUT"
    );
}

#[nats_micro::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_macro_accepts_explicit_multi_thread_options() {
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    );
}
