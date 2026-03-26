use nats_micro::{NatsErrorResponse, SubjectParam, endpoint};

#[endpoint(subject = "test.{a}.profile", group = "test")]
async fn extra_param(
    _a: SubjectParam<String>,
    _b: SubjectParam<String>,
) -> Result<(), NatsErrorResponse> {
    Ok(())
}

fn main() {}