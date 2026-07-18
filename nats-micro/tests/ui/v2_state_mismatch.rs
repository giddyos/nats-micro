use nats_micro::service;

struct ExpectedState;
struct WrongState;

#[service(name = "demo", version = "1.0.0", state = ExpectedState)]
impl Demo {
    #[request]
    async fn get(state: &WrongState) {
        let _ = state;
    }
}

fn main() {}
