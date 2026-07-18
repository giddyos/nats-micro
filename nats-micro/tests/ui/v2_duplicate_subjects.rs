use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("same")]
    async fn one() {}

    #[request("same")]
    async fn two() {}
}

fn main() {}
