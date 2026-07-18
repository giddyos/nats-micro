use nats_micro::{Body, ConsumerAction, service};

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[consumer(
        stream = "EVENTS",
        durable = "demo",
        filter = "events.demo",
        backoff = ["0ms"]
    )]
    async fn consume(body: Body<'_>) -> ConsumerAction {
        let _ = body;
        ConsumerAction::Ack
    }
}

fn main() {}
