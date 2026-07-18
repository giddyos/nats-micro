use nats_micro::{SubjectParam, service};

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request("users")]
    async fn get(user_id: SubjectParam<u64>) {
        let _ = user_id;
    }
}

fn main() {}
