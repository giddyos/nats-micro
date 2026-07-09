#![allow(clippy::unused_async)]

use nats_micro::{Json, NatsErrorResponse, Payload, service, service_error, service_handlers};

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct NapiCreateUserRequest {
    pub email: String,
}

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct NapiUserCreated {
    pub user_id: String,
}

#[service_error]
pub enum NapiUserError {
    #[code(409)]
    #[error("email exists")]
    EmailExists,
}

#[service(name = "napi-user-service", version = "1.0.0", napi = true)]
pub struct NapiUserService;

#[service_handlers]
impl NapiUserService {
    #[endpoint(subject = "users", group = "accounts")]
    async fn create(
        payload: Payload<Json<NapiCreateUserRequest>>,
    ) -> Result<Json<NapiUserCreated>, NapiUserError> {
        Ok(Json(NapiUserCreated {
            user_id: payload.email.clone(),
        }))
    }
}

fn assert_napi_surface() {
    let _ = NapiUserServiceClientConnectOptions::default();
    let _ = JsNapiUserServiceClient::connect;
    let _ = NapiUserServiceClientHeader {
        name: "x-request-id".to_string(),
        value: "req-1".to_string(),
        encrypted: false,
    };
    let _kind = JsNapiUserError::EMAIL_EXISTS;
    let _ = NatsErrorResponse::new(500, "INTERNAL", "internal", "");
}

fn main() {
    assert_napi_surface();
}
