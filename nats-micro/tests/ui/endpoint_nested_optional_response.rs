#![allow(dead_code, unused_imports)]

use nats_micro::{NatsErrorResponse, service, service_handlers};

#[service(name = "bad_response_svc", version = "0.1.0")]
struct BadResponseService;

#[service_handlers]
impl BadResponseService {
    #[endpoint(subject = "maybe")]
    async fn maybe_response() -> Result<Option<Option<String>>, NatsErrorResponse> {
        Ok(None)
    }
}

fn main() {}