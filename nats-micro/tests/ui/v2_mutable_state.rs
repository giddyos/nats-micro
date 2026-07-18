use nats_micro::service;

struct State;

#[service(name = "demo", version = "1.0.0", state = State)]
impl Demo {
    #[request]
    async fn get(state: &mut State) {
        let _ = state;
    }
}

fn main() {}
