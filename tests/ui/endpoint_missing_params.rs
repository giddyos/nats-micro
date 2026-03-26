use nats_micro::{NatsErrorResponse, State, endpoint};

#[endpoint(subject = "test.{a}.{b}.{c}.{d}.profile", group = "test")]
async fn missing_params(_state: State<()>) -> Result<(), NatsErrorResponse> {
    Ok(())
}

fn main() {}