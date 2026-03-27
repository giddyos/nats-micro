use nats_micro::{NatsErrorResponse, consumer};

#[consumer(stream = "EVENTS", durable = "loose-consumer", filter_subject = "events.loose")]
async fn loose_consumer() -> Result<(), NatsErrorResponse> {
    Ok(())
}

fn main() {}