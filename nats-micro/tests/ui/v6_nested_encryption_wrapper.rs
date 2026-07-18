use nats_micro::{Json, service};

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn secret(input: Json<Encrypted<String>>) -> String {
        input.0.into_inner()
    }
}

fn main() {}
