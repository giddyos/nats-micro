use nats_micro::{Auth, service};

struct Claims;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request(auth = optional)]
    async fn me(auth: Auth<Claims>) {
        let _ = auth;
    }
}

fn main() {}
