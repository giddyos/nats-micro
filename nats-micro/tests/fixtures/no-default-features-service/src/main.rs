#![allow(clippy::unused_async)]

use nats_micro::{
    Json, NatsAppConfig, NatsErrorResponse, Payload, SubjectParam, service, service_error,
    service_handlers,
};

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct BasicRequest {
    pub value: String,
}

#[service_error]
pub enum BasicError {
    #[code(400)]
    #[error("invalid value")]
    InvalidValue,
}

#[service(name = "basic", version = "1.0.0")]
pub struct BasicService;

#[service_handlers]
impl BasicService {
    #[endpoint(subject = "echo", group = "basic")]
    async fn echo(payload: Payload<Json<BasicRequest>>) -> Result<Json<BasicRequest>, BasicError> {
        Ok(payload.into_wrapped())
    }

    #[endpoint(subject = "raw.{value}", group = "basic")]
    async fn raw(value: SubjectParam<String>) -> Result<String, NatsErrorResponse> {
        Ok(value.into_inner())
    }
}

#[test]
fn no_default_features_service_surface_works() {
    let _config = NatsAppConfig::new().with_default_concurrency_limit(8);
    let contract = BasicService::contract();
    assert_eq!(contract.endpoints.len(), 2);
    assert!(BasicService::contract_json().unwrap().contains("\"basic\""));
}

fn main() {}
