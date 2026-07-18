use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn secret(input: Encrypted<String>) -> String {
        input.into_inner()
    }
}

fn main() {}
