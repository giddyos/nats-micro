use nats_micro::{message, service};

#[message]
struct User {
    id: String,
}

struct State {
    user: User,
}

#[service(name = "demo", version = "1.0.0", state = State)]
impl Demo {
    #[request]
    async fn get<'a>(state: &'a State) -> &'a User {
        &state.user
    }
}

fn main() {}
