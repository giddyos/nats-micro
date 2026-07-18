use nats_micro::{application, service};

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

application! {
    state: (),
    profile: Test,
    services: [One, Two],
}
