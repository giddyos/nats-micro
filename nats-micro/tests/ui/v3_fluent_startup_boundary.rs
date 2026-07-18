use nats_micro::{App, service};

#[service(name = "duplicate", version = "1.0.0")]
impl One {
    #[request("shared")]
    async fn shared() {}
}

#[service(name = "duplicate", version = "1.0.0")]
impl Two {
    #[request("shared")]
    async fn shared() {}
}

fn main() {
    // Fluent composition crosses a stable-const boundary. The same duplicate
    // is rejected deterministically by App::start before connecting.
    let _ = App::stateless().serve(One).serve(Two);
}
