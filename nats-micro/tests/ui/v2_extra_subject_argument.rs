use nats_micro::{message, service};

#[message]
struct Input {
    value: String,
}

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("users.{user_id}")]
    async fn get(user_id: &str, input: Input, order_id: u64) {
        let _ = (user_id, input, order_id);
    }
}

fn main() {}
