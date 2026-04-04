use nats_micro::{NatsErrorResponse, SubjectParam, service, service_handlers};

#[service(name = "extra-param")]
struct ExtraParamService;

#[service_handlers]
impl ExtraParamService {
    #[endpoint(subject = "test.{a}.profile", group = "test")]
    async fn extra_param(
        _a: SubjectParam<String>,
        _b: SubjectParam<String>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}