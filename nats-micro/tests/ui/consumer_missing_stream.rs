use nats_micro::{service, service_handlers};

#[service(name = "missing_stream", version = "1.0.0", default_stream = "EVENTS")]
struct MissingStreamService;

#[service_handlers]
impl MissingStreamService {
    #[consumer(durable = "jobs")]
    async fn jobs() -> Result<(), nats_micro::NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}
