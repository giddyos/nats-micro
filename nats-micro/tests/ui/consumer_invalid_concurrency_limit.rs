use nats_micro::{service, service_handlers};

#[service(name = "svc", version = "1.0.0")]
struct InvalidConsumerConcurrencyService;

#[service_handlers]
impl InvalidConsumerConcurrencyService {
    #[consumer(
        stream = "EVENTS",
        durable = "invalid-consumer-concurrency",
        concurrency_limit = "many"
    )]
    async fn handle_event() -> Result<(), nats_micro::NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}