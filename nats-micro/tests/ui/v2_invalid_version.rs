use nats_micro::service;

#[service(name = "demo", version = "1.2")]
impl Demo {
    #[request]
    async fn health() {}
}

fn main() {}
