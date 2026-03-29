use nats_micro::{
    Json, NatsErrorResponse, Payload,
    service, service_handlers,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct First {
    a: i64,
}

#[derive(Debug, Deserialize)]
struct Second {
    b: String,
}

#[service(name = "bad_svc", version = "0.1.0")]
struct BadService;

#[service_handlers]
impl BadService {
    #[endpoint(subject = "double")]
    async fn double_payload(
        first: Payload<Json<First>>,
        second: Payload<Json<Second>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok("should not compile".to_string())
    }
}

fn main() {}
