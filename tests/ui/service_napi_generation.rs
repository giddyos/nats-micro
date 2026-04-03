use nats_micro::{Json, NatsErrorResponse, Payload, SubjectParam, service, service_handlers};
#[cfg(feature = "napi")]
use nats_micro::napi;
use serde::{Deserialize, Serialize};

#[nats_micro::object]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SumRequest {
    pub numbers: Vec<i64>,
}

#[nats_micro::object]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SumResponse {
    pub total: i64,
}

#[nats_micro::object]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
}

#[service(name = "demo_svc", version = "1.0.0", napi = true)]
struct DemoService;

#[service_handlers]
impl DemoService {
    #[endpoint(subject = "sum", group = "math")]
    async fn sum(
        payload: Payload<Json<SumRequest>>,
    ) -> Result<Json<SumResponse>, NatsErrorResponse> {
        Ok(Json(SumResponse {
            total: payload.numbers.iter().sum(),
        }))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "accounts")]
    async fn get_profile(
        user_id: SubjectParam<String>,
    ) -> Result<Json<UserProfile>, NatsErrorResponse> {
        Ok(Json(UserProfile {
            user_id: user_id.to_string(),
        }))
    }

    #[endpoint(subject = "health")]
    async fn health() -> Result<String, NatsErrorResponse> {
        Ok("ok".to_string())
    }

    #[endpoint(subject = "echo", group = "raw")]
    async fn echo(payload: Payload<Vec<u8>>) -> Result<Vec<u8>, NatsErrorResponse> {
        Ok(payload.0)
    }
}

#[cfg(feature = "napi")]
fn _assert_connect() {
    let _ = DemoServiceClientConnectOptions::default();
    let _ = JsDemoServiceClient::connect;
}

#[cfg(feature = "napi")]
fn _assert_sum(client: &JsDemoServiceClient) {
    let _ = async {
        let payload = SumRequest {
            numbers: vec![1, 2, 3],
        };
        let _result: nats_micro::napi::Result<SumResponse> = client.sum(payload).await;
    };
}

#[cfg(feature = "napi")]
fn _assert_profile(client: &JsDemoServiceClient) {
    let _ = async {
        let args = DemoServiceGetProfileArgs {
            user_id: "alice".to_string(),
        };
        let _result: nats_micro::napi::Result<UserProfile> = client.get_profile(args).await;
    };
}

#[cfg(feature = "napi")]
fn _assert_health(client: &JsDemoServiceClient) {
    let _ = async {
        let _result: nats_micro::napi::Result<String> = client.health().await;
    };
}

#[cfg(feature = "napi")]
fn _assert_echo(client: &JsDemoServiceClient) {
    let _ = async {
        let payload = nats_micro::napi::bindgen_prelude::Buffer::from(vec![1u8, 2, 3]);
        let _result: nats_micro::napi::Result<nats_micro::napi::bindgen_prelude::Buffer> =
            client.echo(payload).await;
    };
}

fn main() {}