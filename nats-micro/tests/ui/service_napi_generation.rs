use nats_micro::{Json, NatsErrorResponse, Payload, SubjectParam, service, service_handlers};
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
    let _ = DemoServiceClientConnectOptions {
        headers: Some(demo_headers()),
        ..Default::default()
    };
    let _ = JsDemoServiceClient::connect;
}

#[cfg(feature = "napi")]
fn demo_headers() -> Vec<DemoServiceClientHeader> {
    vec![
        DemoServiceClientHeader {
            name: "x-request-id".to_string(),
            value: "req-1".to_string(),
            encrypted: false,
        },
        DemoServiceClientHeader {
            name: "x-tenant-id".to_string(),
            value: "tenant-42".to_string(),
            encrypted: true,
        },
    ]
}

#[cfg(feature = "napi")]
fn _assert_sum(client: &JsDemoServiceClient) {
    let _ = async {
        let payload = SumRequest {
            numbers: vec![1, 2, 3],
        };
        let _result = client.sum(payload).await;
        let payload = SumRequest {
            numbers: vec![1, 2, 3],
        };
        let _result = client.sum_with_headers(payload, Some(demo_headers())).await;
    };
}

#[cfg(feature = "napi")]
fn _assert_profile(client: &JsDemoServiceClient) {
    let _ = async {
        let args = DemoServiceGetProfileArgs {
            user_id: "alice".to_string(),
        };
        let _result = client.get_profile(args).await;
        let args = DemoServiceGetProfileArgs {
            user_id: "alice".to_string(),
        };
        let _result = client
            .get_profile_with_headers(args, Some(demo_headers()))
            .await;
    };
}

#[cfg(feature = "napi")]
fn _assert_health(client: &JsDemoServiceClient) {
    let _ = async {
        let _result = client.health().await;
        let _result = client.health_with_headers(Some(demo_headers())).await;
    };
}

#[cfg(feature = "napi")]
fn _assert_echo(client: &JsDemoServiceClient) {
    let _ = async {
        let payload = nats_micro::napi::bindgen_prelude::Buffer::from(vec![1u8, 2, 3]);
        let _result = client.echo(payload).await;
        let payload = nats_micro::napi::bindgen_prelude::Buffer::from(vec![1u8, 2, 3]);
        let _result = client.echo_with_headers(payload, Some(demo_headers())).await;
    };
}

#[cfg(feature = "napi")]
fn _assert_set_headers(client: &mut JsDemoServiceClient) {
    client.set_headers(Some(demo_headers()));
    client.set_headers(None);
}

fn main() {}