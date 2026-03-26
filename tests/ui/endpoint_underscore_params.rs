use nats_micro::{NatsErrorResponse, SubjectParam, endpoint};

#[endpoint(subject = "test.{a}.{b}.profile", group = "test")]
async fn underscore_params(
    _a: SubjectParam<String>,
    _b: SubjectParam<u32>,
) -> Result<(), NatsErrorResponse> {
    Ok(())
}

fn main() {}