use nats_micro::{
    FromSubjectParam, NatsErrorResponse, SubjectParam, service, service_handlers,
};

struct BoolFromDigits(bool);

impl FromSubjectParam for BoolFromDigits {
    type Err = &'static str;

    fn from_subject_param(value: &str) -> Result<Self, Self::Err> {
        match value {
            "1" => Ok(Self(true)),
            "0" => Ok(Self(false)),
            _ => Err("expected 0 or 1"),
        }
    }
}

#[service(name = "custom-param")]
struct CustomParamService;

#[service_handlers]
impl CustomParamService {
    #[endpoint(subject = "test.{enabled}.profile", group = "test")]
    async fn custom_param(
        _enabled: SubjectParam<BoolFromDigits>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {}