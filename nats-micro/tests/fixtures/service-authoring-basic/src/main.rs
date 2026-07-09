#![allow(clippy::unused_async)]

use nats_micro::prelude::*;
use nats_micro::{FromNatsErrorResponse, ServiceErrorMatch};

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct CreateUserRequest {
    pub email: String,
}

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct UserSnapshot {
    pub user_id: String,
    pub email: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserCreated {
    pub user_id: String,
}

impl nats_micro::prost::Message for UserCreated {
    fn encode_raw(&self, buf: &mut impl nats_micro::prost::bytes::BufMut) {
        nats_micro::prost::encoding::string::encode(1, &self.user_id, buf);
    }

    fn merge_field(
        &mut self,
        tag: u32,
        wire_type: nats_micro::prost::encoding::WireType,
        buf: &mut impl nats_micro::prost::bytes::Buf,
        ctx: nats_micro::prost::encoding::DecodeContext,
    ) -> Result<(), nats_micro::prost::DecodeError> {
        match tag {
            1 => nats_micro::prost::encoding::string::merge(
                wire_type,
                &mut self.user_id,
                buf,
                ctx,
            ),
            _ => nats_micro::prost::encoding::skip_field(wire_type, tag, buf, ctx),
        }
    }

    fn encoded_len(&self) -> usize {
        nats_micro::prost::encoding::string::encoded_len(1, &self.user_id)
    }

    fn clear(&mut self) {
        self.user_id.clear();
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    pub shard: String,
}

pub struct UserClaims {
    pub subject: String,
}

impl FromAuthRequest for UserClaims {
    async fn from_auth_request(ctx: &RequestContext) -> Result<Self, AuthError> {
        let subject = ctx
            .request
            .headers()
            .get("authorization")
            .map(|value| value.as_str().to_string())
            .ok_or(AuthError::MissingCredentials)?;
        Ok(Self { subject })
    }
}

#[nats_micro::service_error]
pub enum UserServiceError {
    #[code(409)]
    #[error("email {email} already exists")]
    EmailExists { email: String },

    #[code(422)]
    #[error("invalid field {field:?}: {reason}")]
    InvalidField { field: String, reason: String },

    #[code(429)]
    #[error("rate limited for {retry_after:?}")]
    RateLimited { retry_after: u64 },

    #[code(503)]
    #[error("failed to create user {email}: {source}")]
    CreateFailed {
        email: String,
        #[source]
        source: std::io::Error,
    },

    #[error("storage failed")]
    Storage(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] std::num::ParseIntError),
}

#[nats_micro::service(
    name = "user-service",
    version = "1.0.0",
    description = "Downstream service authoring fixture"
)]
pub struct UserService;

#[nats_micro::service_handlers]
impl UserService {
    #[endpoint(subject = "users", group = "accounts")]
    async fn create_user(
        payload: Payload<Json<CreateUserRequest>>,
        state: State<AppState>,
        headers: Headers,
        request_id: RequestId,
        subject: Subject,
        _auth: Auth<UserClaims>,
    ) -> Result<Json<UserSnapshot>, UserServiceError> {
        let tenant = headers
            .get("x-tenant-id")
            .map(Header::as_str)
            .unwrap_or("default");
        Ok(Json(UserSnapshot {
            user_id: format!("{}:{}:{}", state.shard, tenant, request_id.as_inner()),
            email: format!("{}@{}", payload.email, subject.as_inner()),
        }))
    }

    #[endpoint(subject = "users.created", group = "accounts")]
    async fn create_user_proto(
        payload: Payload<Proto<UserCreated>>,
    ) -> Result<Proto<UserCreated>, NatsErrorResponse> {
        Ok(payload.into_wrapped())
    }

    #[endpoint(subject = "echo", group = "raw")]
    async fn echo_string(payload: Payload<String>) -> Result<String, NatsErrorResponse> {
        Ok(payload.into_inner())
    }

    #[endpoint(subject = "maybe", group = "raw")]
    async fn optional_payload(
        payload: Payload<Option<Json<CreateUserRequest>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .into_inner()
            .map(|payload| payload.email)
            .unwrap_or_else(|| "none".to_string()))
    }

    #[endpoint(subject = "maybe-response", group = "raw")]
    async fn optional_response(
        payload: Payload<String>,
    ) -> Result<Option<String>, NatsErrorResponse> {
        let value = payload.into_inner();
        Ok((value != "none").then_some(value))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "accounts")]
    async fn subject_param(
        user_id: SubjectParam<String>,
        _auth: Option<Auth<UserClaims>>,
    ) -> Result<Json<UserSnapshot>, NatsErrorResponse> {
        Ok(Json(UserSnapshot {
            user_id: user_id.into_inner(),
            email: "known@example.com".to_string(),
        }))
    }

    #[endpoint(subject = "public", group = "accounts")]
    async fn public_health() -> Result<(), NatsErrorResponse> {
        Ok(())
    }

    #[consumer(
        stream = "USERS",
        durable = "user-service-events",
        concurrency_limit = 4
    )]
    async fn user_events(
        _payload: Payload<String>,
        _state: State<AppState>,
        _headers: Headers,
        _request_id: RequestId,
        _auth: Option<Auth<UserClaims>>,
        _shutdown: ShutdownSignal,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn assert_service_error_wire() {
    let err = UserServiceError::EmailExists {
        email: "a@b.com".to_string(),
    };
    let response = err.into_nats_error("req-1".to_string());

    assert_eq!(response.code, 409);
    assert_eq!(response.kind, "EMAIL_EXISTS");
    assert_eq!(response.message, "email a@b.com already exists");
    assert_eq!(response.request_id, "req-1");
    assert!(response.details.is_some());

    match <UserServiceError as FromNatsErrorResponse>::from_nats_error_response(response) {
        ServiceErrorMatch::Typed(UserServiceError::EmailExists { email }) => {
            assert_eq!(email, "a@b.com");
        }
        other => panic!("expected typed EmailExists, got {other:?}"),
    }

    let err = UserServiceError::from(std::io::Error::other("disk"));
    assert!(std::error::Error::source(&err).is_some());
    let response = err.into_nats_error("req-2".to_string());

    assert_eq!(response.code, 500);
    assert_eq!(response.message, "an internal error occurred");
    assert!(response.details.is_none());

    let err = UserServiceError::RateLimited { retry_after: 30 };
    let response = err.into_nats_error("req-3".to_string());
    assert_eq!(response.code, 429);
    assert_eq!(
        response
            .details
            .as_ref()
            .and_then(|details| details.get("retry_after"))
            .and_then(nats_micro::serde_json::Value::as_u64),
        Some(30)
    );

    let err = UserServiceError::CreateFailed {
        email: "a@b.com".to_string(),
        source: std::io::Error::other("disk"),
    };
    assert!(std::error::Error::source(&err).is_some());
    let response = err.into_nats_error("req-4".to_string());
    assert_eq!(
        response
            .details
            .as_ref()
            .and_then(|details| details.get("email"))
            .and_then(nats_micro::serde_json::Value::as_str),
        Some("a@b.com")
    );
    assert!(matches!(
        <UserServiceError as FromNatsErrorResponse>::from_nats_error_response(response),
        ServiceErrorMatch::Untyped(_)
    ));

    let parse_error = "not-a-number"
        .parse::<u64>()
        .expect_err("fixture parse error");
    let err = UserServiceError::from(parse_error);
    assert!(std::error::Error::source(&err).is_some());
    let response = err.into_nats_error("req-5".to_string());
    assert_eq!(response.code, 500);
    assert_eq!(response.message, "an internal error occurred");
}

fn assert_contract() {
    let definition = UserService::definition();
    assert_eq!(
        definition.metadata.subject_prefix,
        Some("user-service".to_string())
    );

    let endpoints = UserService::endpoints();
    assert_eq!(
        endpoints.create_user.full_subject_template(),
        "user-service.v1.accounts.users"
    );
    let contract = UserService::contract();
    assert_eq!(contract.endpoints.len(), 7);
    assert_eq!(contract.consumers.len(), 1);
    assert_eq!(contract.endpoints[0].auth_policy, AuthPolicy::Required);
    assert_eq!(
        contract
            .endpoints
            .iter()
            .find(|endpoint| endpoint.fn_name == "subject_param")
            .expect("subject_param endpoint")
            .auth_policy,
        AuthPolicy::Optional
    );
    assert_eq!(
        contract
            .endpoints
            .iter()
            .find(|endpoint| endpoint.fn_name == "public_health")
            .expect("public_health endpoint")
            .auth_policy,
        AuthPolicy::None
    );
    assert_eq!(contract.consumers[0].auth_policy, AuthPolicy::Optional);

    let json = UserService::contract_json().expect("contract should serialize");
    assert!(json.contains("\"create_user\""));
    assert!(json.contains("\"user_events\""));
    assert!(json.contains("\"auth_policy\": \"required\""));
    assert!(json.contains("\"auth_policy\": \"optional\""));
    assert!(json.contains("\"auth_policy\": \"none\""));
}

fn assert_prelude_surface() {
    let _headers = Headers::new();
    let _config = NatsAppConfig::new().with_default_concurrency_limit(32);
    let _policy = WorkerFailurePolicy::Ignore;
}

fn main() {
    assert_service_error_wire();
    assert_contract();
    assert_prelude_surface();
}
