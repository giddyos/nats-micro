use nats_micro::{
    Json, NatsErrorResponse, NatsService, Payload, PayloadEncoding,
    ResponseEncoding, SubjectParam, service, service_handlers,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct SumRequest {
    numbers: Vec<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SumResponse {
    total: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct UserProfile {
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HealthStatus {
    ok: bool,
}

struct AppState;

#[service(name = "test_svc", version = "1.0.0", description = "Test")]
struct TestService;

#[service_handlers]
impl TestService {
    #[endpoint(subject = "sum", group = "math")]
    async fn sum_numbers(
        payload: Payload<Json<SumRequest>>,
    ) -> Result<Json<SumResponse>, NatsErrorResponse> {
        Ok(Json(SumResponse {
            total: payload.numbers.iter().sum(),
        }))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "accounts")]
    async fn get_user_profile(
        user_id: SubjectParam<String>,
    ) -> Result<Json<UserProfile>, NatsErrorResponse> {
        Ok(Json(UserProfile {
            user_id: user_id.to_string(),
        }))
    }

    #[endpoint(subject = "health")]
    async fn health() -> Result<Json<HealthStatus>, NatsErrorResponse> {
        Ok(Json(HealthStatus { ok: true }))
    }

    #[endpoint(subject = "inspect", group = "debug")]
    async fn inspect_request() -> Result<String, NatsErrorResponse> {
        Ok("inspected".to_string())
    }

    #[endpoint(subject = "echo", group = "raw")]
    async fn echo(payload: Payload<String>) -> Result<String, NatsErrorResponse> {
        Ok(payload.0)
    }
}

fn main() {
    let def = TestService::definition();

    let sum_info = def
        .endpoint_info
        .iter()
        .find(|i| i.fn_name == "sum_numbers")
        .unwrap();
    let pm = sum_info.payload_meta.as_ref().unwrap();
    assert_eq!(pm.encoding, PayloadEncoding::Json);
    assert!(!pm.encrypted);
    assert_eq!(sum_info.response_meta.encoding, ResponseEncoding::Json);
    assert!(!sum_info.response_meta.encrypted);

    let profile_info = def
        .endpoint_info
        .iter()
        .find(|i| i.fn_name == "get_user_profile")
        .unwrap();
    assert!(profile_info.payload_meta.is_none());
    assert_eq!(profile_info.response_meta.encoding, ResponseEncoding::Json);

    let health_info = def
        .endpoint_info
        .iter()
        .find(|i| i.fn_name == "health")
        .unwrap();
    assert!(health_info.payload_meta.is_none());
    assert_eq!(health_info.response_meta.encoding, ResponseEncoding::Json);

    let inspect_info = def
        .endpoint_info
        .iter()
        .find(|i| i.fn_name == "inspect_request")
        .unwrap();
    assert!(inspect_info.payload_meta.is_none());
    assert_eq!(inspect_info.response_meta.encoding, ResponseEncoding::Raw);

    let echo_info = def
        .endpoint_info
        .iter()
        .find(|i| i.fn_name == "echo")
        .unwrap();
    let echo_pm = echo_info.payload_meta.as_ref().unwrap();
    assert_eq!(echo_pm.encoding, PayloadEncoding::Raw);
    assert_eq!(echo_info.response_meta.encoding, ResponseEncoding::Raw);
}
