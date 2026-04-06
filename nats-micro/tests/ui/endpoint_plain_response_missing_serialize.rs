#![allow(dead_code, unused_imports)]

use nats_micro::{NatsErrorResponse, service, service_handlers};

struct MissingSerialize;

#[service(name = "bad_plain_response_svc", version = "0.1.0")]
struct BadPlainResponseService;

#[service_handlers]
impl BadPlainResponseService {
    #[endpoint(subject = "bad")]
    async fn bad() -> Result<Vec<MissingSerialize>, NatsErrorResponse> {
        Ok(Vec::new())
    }
}

fn main() {}
