use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("bad subject")]
    async fn bad() {}
}

fn main() {}
