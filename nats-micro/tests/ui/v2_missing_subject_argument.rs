use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("users.{user_id}")]
    async fn get() {}
}

fn main() {}
