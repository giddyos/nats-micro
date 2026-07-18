use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request(auth = required)]
    async fn me() {}
}

fn main() {}
