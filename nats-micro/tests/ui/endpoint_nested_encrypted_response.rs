#![allow(dead_code, unused_imports)]

use nats_micro::{Encrypted, Json, NatsErrorResponse, service, service_handlers};

#[service(name = "bad_encrypted_response_svc", version = "0.1.0")]
struct BadEncryptedResponseService;

#[service_handlers]
impl BadEncryptedResponseService {
    #[endpoint(subject = "bad")]
    async fn bad() -> Result<Json<Encrypted<String>>, NatsErrorResponse> {
        Ok(Json(Encrypted(String::new())))
    }
}

fn main() {}
