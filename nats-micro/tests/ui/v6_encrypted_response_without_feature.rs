use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn secret() -> Encrypted<String> {
        unreachable!()
    }
}

fn main() {}
