use nats_micro::{application, service};

#[service(name = "one", version = "1.0.0")]
impl One {
    #[request("shared")]
    async fn shared() {}
}

#[service(name = "two", version = "1.0.0")]
impl Two {
    #[request("shared")]
    async fn shared() {}
}

fn main() {
    // Cross-service duplicates remain a generated-application/startup boundary
    // when stable const evaluation cannot report both declaration spans.
    let _ = application!(One, Two);
}
