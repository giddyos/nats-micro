#![allow(clippy::redundant_closure_for_method_calls, clippy::unused_async)]

use nats_micro::prelude::*;
use nats_micro::{service, service_handlers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreludeRequest {
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreludeResponse {
    value: String,
    id: u64,
}

#[derive(Clone)]
struct PreludeState;

#[service(name = "prelude-smoke", version = "1.0.0")]
struct PreludeService;

#[service_handlers]
impl PreludeService {
    #[endpoint(subject = "echo.{id}")]
    async fn echo(
        state: State<PreludeState>,
        id: SubjectParam<u64>,
        payload: Payload<Json<PreludeRequest>>,
        request_id: RequestId,
    ) -> Result<Json<PreludeResponse>, NatsErrorResponse> {
        let _ = state;
        let _ = request_id.as_inner();
        Ok(Json(PreludeResponse {
            value: payload.as_inner().as_inner().value.clone(),
            id: id.into_inner(),
        }))
    }
}

fn request_context() -> RequestContext {
    let mut headers = Headers::new();
    headers.insert("x-request-id", "req-prelude");

    RequestContext::new(
        NatsRequest {
            subject: "echo.7".to_string(),
            payload: Bytes::new(),
            headers,
            reply: None,
            request_id: "req-prelude".to_string(),
        },
        StateMap::new().insert(PreludeState),
        Some("echo.{id}".to_string()),
    )
}

#[test]
fn prelude_reexports_service_authoring_surface() {
    let def = PreludeService::definition();
    assert_eq!(def.metadata.name, "prelude-smoke");
    assert_eq!(PreludeService::echo_endpoint().full_subject(), "v1.echo.*");

    let mut headers = Headers::new();
    headers.insert("x-request-id", "req-prelude");
    assert_eq!(
        headers.get("x-request-id").map(Header::as_str),
        Some("req-prelude")
    );

    let payload = Payload(Json(PreludeRequest {
        value: "hello".to_string(),
    }));
    assert_eq!(payload.as_inner().as_inner().value, "hello");

    let config = NatsAppConfig::new()
        .with_default_concurrency_limit(32)
        .with_worker_failure_policy(WorkerFailurePolicy::Ignore);
    assert_eq!(config.default_concurrency_limit(), 32);

    let response = Json(PreludeResponse {
        value: "ok".to_string(),
        id: 7,
    })
    .into_response(&request_context())
    .expect("json response should serialize");
    assert!(!response.payload.is_empty());
}

#[tokio::test]
async fn prelude_extractors_work_in_external_tests() {
    let ctx = request_context().with_param_name("id");

    let id = SubjectParam::<u64>::from_request(&ctx)
        .await
        .expect("subject param should extract");
    let request_id = RequestId::from_request(&ctx)
        .await
        .expect("request id should extract");
    let subject = Subject::from_request(&ctx)
        .await
        .expect("subject should extract");

    assert_eq!(id.into_inner(), 7);
    assert_eq!(request_id.as_inner(), "req-prelude");
    assert_eq!(subject.as_inner(), "echo.7");
}

#[cfg(feature = "client")]
#[test]
fn prelude_reexports_client_options() {
    let options = ClientCallOptions::new().header("x-trace", "123");
    assert_eq!(
        options
            .plaintext_headers
            .get("x-trace")
            .map(|value| value.as_str()),
        Some("123")
    );
}

#[cfg(feature = "encryption")]
#[test]
fn prelude_reexports_encryption_types() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let BuiltRequest {
        headers,
        payload: _,
        context,
    } = recipient
        .request_builder()
        .encrypted_header("authorization", "Bearer demo-token")
        .build()
        .expect("encrypted request should build");

    let shared_key = keypair.derive_shared_key(&context.ephemeral_pub_bytes());
    let decrypted_headers = nats_micro::encrypted_headers_decrypt(&headers, &shared_key)
        .expect("encrypted headers should decrypt");

    assert_eq!(
        decrypted_headers.get("authorization"),
        Some(&"Bearer demo-token".to_string())
    );

    let encrypted = Encrypted(String::from("payload"));
    assert_eq!(encrypted.as_inner(), "payload");
}
