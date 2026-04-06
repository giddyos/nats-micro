use nats_micro::{NatsErrorResponse, NatsService, service, service_handlers};

#[service(name = "prefixed", version = "1.0.0", prefix = "api")]
struct PrefixedService;

#[service_handlers]
impl PrefixedService {
    #[endpoint(subject = "sum", group = "math")]
    async fn sum() -> Result<String, NatsErrorResponse> {
        Ok("ok".to_string())
    }

    #[endpoint(subject = "health")]
    async fn health() -> Result<String, NatsErrorResponse> {
        Ok("ok".to_string())
    }
}

fn main() {
    let def = PrefixedService::definition();
    assert_eq!(def.metadata.subject_prefix.as_deref(), Some("api"));
    assert_eq!(PrefixedService::sum_endpoint().full_subject(), "api.v1.math.sum");
    assert_eq!(PrefixedService::health_endpoint().full_subject(), "api.v1.health");
}