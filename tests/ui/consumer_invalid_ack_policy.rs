use nats_micro::{NatsErrorResponse, consumer, service, service_handlers};

#[service(name = "svc")]
struct InvalidConsumerService;

#[service_handlers]
impl InvalidConsumerService {
    #[consumer(stream = "EVENTS", durable = "invalid-consumer", filter_subject = "events.invalid", ack_policy = Bogus)]
    async fn handle_event() -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}