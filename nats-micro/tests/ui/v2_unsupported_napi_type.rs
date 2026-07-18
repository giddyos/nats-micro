use std::collections::HashMap;

use nats_micro::service;

#[service(name = "demo", version = "1.0.0", napi)]
impl Demo {
    #[request]
    async fn map() -> HashMap<String, String> {
        HashMap::new()
    }
}

fn main() {}
