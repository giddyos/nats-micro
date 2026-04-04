use nats_micro::{NatsErrorResponse, SubjectParam, service, service_handlers};

#[service(name = "underscore-params")]
struct UnderscoreParamsService;

#[service_handlers]
impl UnderscoreParamsService {
    #[endpoint(subject = "test.{a}.{b}.profile", group = "test")]
    async fn underscore_params(
        _a: SubjectParam<String>,
        _b: SubjectParam<u32>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}