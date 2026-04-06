use nats_micro::{NatsErrorResponse, service, service_handlers};

#[service(name = "svc", version = "1.0.0")]
struct InvalidConsumerConfigService;

#[service_handlers]
impl InvalidConsumerConfigService {
    #[consumer(
        stream = "EVENTS",
        durable = "invalid-consumer-config",
        config = 123
    )]
    async fn handle_event() -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}