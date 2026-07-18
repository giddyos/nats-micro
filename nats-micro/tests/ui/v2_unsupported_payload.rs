use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn create(callback: fn()) {
        callback();
    }
}

fn main() {}
