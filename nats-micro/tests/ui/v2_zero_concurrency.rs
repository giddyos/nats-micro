use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request(concurrency = 0)]
    async fn get() {}
}

fn main() {}
