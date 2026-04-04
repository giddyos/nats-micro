#![allow(dead_code, unused_imports)]

use nats_micro::{NatsErrorResponse, Payload, service, service_handlers};

#[service(name = "bad_svc", version = "0.1.0")]
struct BadService;

#[service_handlers]
impl BadService {
    #[endpoint(subject = "maybe")]
    async fn maybe_payload(
        payload: Payload<Option<Option<String>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload.0.flatten().unwrap_or_default())
    }
}

fn main() {}