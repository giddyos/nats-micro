use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn create(first: String, second: String) {
        let _ = (first, second);
    }
}

fn main() {}
