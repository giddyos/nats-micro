use nats_micro::service;

#[service(name = "demo", version = "1.0.0", napi)]
impl Demo {
    #[request]
    async fn ping() {}
}

fn main() {}
