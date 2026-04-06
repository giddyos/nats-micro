use nats_micro::{NatsErrorResponse, State, service, service_handlers};

#[service(name = "missing-params", version = "1.0.0")]
struct MissingParamsService;

#[service_handlers]
impl MissingParamsService {
    #[endpoint(subject = "test.{a}.{b}.{c}.{d}.profile", group = "test")]
    async fn missing_params(_state: State<()>) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}