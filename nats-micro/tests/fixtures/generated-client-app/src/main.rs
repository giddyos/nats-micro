#![allow(clippy::unused_async)]

use nats_micro::{
    ClientCallOptions, ClientError, IntoNatsError, Json, NatsErrorResponse, Payload, RequestContext,
    SubjectParam, service, service_error, service_handlers,
};

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct CreateUserRequest {
    pub email: String,
}

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct UserCreated {
    pub user_id: String,
}

#[service_error]
pub enum UserServiceError {
    #[code(409)]
    #[error("email exists")]
    EmailExists,
}

#[service(name = "user-service", version = "1.0.0")]
pub struct UserService;

#[service_handlers]
impl UserService {
    #[endpoint(subject = "health")]
    async fn health() -> Result<String, UserServiceError> {
        Ok("ok".to_string())
    }

    #[endpoint(subject = "users", group = "accounts")]
    async fn create(
        payload: Payload<Json<CreateUserRequest>>,
    ) -> Result<Json<UserCreated>, UserServiceError> {
        Ok(Json(UserCreated {
            user_id: payload.email.clone(),
        }))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "accounts")]
    async fn profile(user_id: SubjectParam<String>) -> Result<Json<UserCreated>, UserServiceError> {
        Ok(Json(UserCreated {
            user_id: user_id.into_inner(),
        }))
    }

    #[endpoint(subject = "maybe-payload", group = "accounts")]
    async fn maybe_payload(
        payload: Payload<Option<Json<CreateUserRequest>>>,
    ) -> Result<String, UserServiceError> {
        Ok(payload
            .into_inner()
            .map(|payload| payload.email)
            .unwrap_or_else(|| "none".to_string()))
    }

    #[endpoint(subject = "maybe-response", group = "accounts")]
    async fn maybe_response(payload: Payload<String>) -> Result<Option<String>, UserServiceError> {
        Ok(Some(payload.into_inner()))
    }

    #[endpoint(subject = "echo-string", group = "raw")]
    async fn echo_string(payload: Payload<String>) -> Result<String, UserServiceError> {
        Ok(payload.into_inner())
    }

    #[endpoint(subject = "echo-bytes", group = "raw")]
    async fn echo_bytes(payload: Payload<Vec<u8>>) -> Result<Vec<u8>, UserServiceError> {
        Ok(payload.into_inner())
    }
}

type CreateResult = Result<UserCreated, ClientError<UserServiceError>>;
type MaybeResult = Result<Option<String>, ClientError<UserServiceError>>;

fn uses_client(client: nats_micro::NatsClient, ctx: &RequestContext) {
    let generated = user_service_client::UserServiceClient::new(client.clone());
    let _prefixed = user_service_client::UserServiceClient::with_prefix(client.clone(), "tenant-a");
    let _built = user_service_client::UserServiceClient::builder(client)
        .prefix("tenant-a")
        .default_header("x-client-name", "fixture")
        .expect("valid header")
        .build();

    let create = CreateUserRequest {
        email: "a@b.com".to_string(),
    };
    let user_id = "user-1".to_string();

    let options = ClientCallOptions::new()
        .header("x-client-name", "fixture")
        .try_header("x-trace-id", "trace-1")
        .expect("valid trace header")
        .bearer_token("plain-token");

    let _ = async move {
        let _health: Result<String, ClientError<UserServiceError>> = generated.health().await;
        let _health_with: Result<String, ClientError<UserServiceError>> =
            generated.health_with(options).await;
        let _health_ctx: Result<String, ClientError<UserServiceError>> =
            generated.health_with_context(ctx).await;
        let _created: CreateResult = generated.create(&create).await;
        let _created_with: CreateResult =
            generated.create_with(&create, ClientCallOptions::new()).await;
        let _created_ctx: CreateResult = generated.create_with_context(ctx, &create).await;
        let _profile: CreateResult = generated.profile(&user_id).await;
        let _profile_with: CreateResult = generated
            .profile_with(&user_id, ClientCallOptions::new())
            .await;
        let _maybe_payload: Result<String, ClientError<UserServiceError>> =
            generated.maybe_payload(Some(&create)).await;
        let _maybe_payload_none: Result<String, ClientError<UserServiceError>> =
            generated.maybe_payload(None).await;
        let _maybe_response: MaybeResult = generated.maybe_response("hello").await;
        let _raw_string: Result<String, ClientError<UserServiceError>> =
            generated.echo_string("hello").await;
        let _raw_bytes: Result<Vec<u8>, ClientError<UserServiceError>> =
            generated.echo_bytes(b"hello").await;
    };
}

#[test]
fn assert_public_error_path() {
    let response = UserServiceError::EmailExists.into_nats_error("req-1".to_string());
    let typed: ClientError<UserServiceError> = ClientError::from_service_response(response);
    assert!(matches!(typed, ClientError::Service { .. }));

    let untyped: ClientError<UserServiceError> =
        ClientError::from_service_response(NatsErrorResponse::new(499, "UNKNOWN", "unknown", ""));
    assert!(matches!(untyped, ClientError::ServiceResponse(_)));
}

#[test]
fn rejects_invalid_header_values() {
    assert!(ClientCallOptions::new()
        .try_header("x-test", "bad\r\nvalue")
        .is_err());
}

fn main() {
    let _ = uses_client;
}
