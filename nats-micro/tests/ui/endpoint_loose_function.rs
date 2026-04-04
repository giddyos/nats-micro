use nats_micro::{NatsErrorResponse, endpoint};

#[endpoint(subject = "test.health", group = "system")]
async fn loose_endpoint() -> Result<(), NatsErrorResponse> {
    Ok(())
}

fn main() {}