use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("users.{user_id}.{user_id}")]
    async fn get(user_id: &str) {
        let _ = user_id;
    }
}

fn main() {}
