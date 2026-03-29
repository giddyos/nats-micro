use nats_micro::{
    Json, NatsErrorResponse, NatsService, Payload, SubjectParam, service,
    service_handlers,
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

#[service(name = "demo_svc", version = "1.0.0")]
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
    async fn echo(payload: Payload<String>) -> Result<String, NatsErrorResponse> {
        Ok(payload.0)
    }
}

#[cfg(feature = "client")]
fn _assert_client_module() {
    use demo_service_client::DemoServiceClient;

    #[cfg(feature = "encryption")]
    fn _assert_constructors(client: nats_micro::async_nats::Client) {
        let recipient = [7u8; 32];
        let _typed = DemoServiceClient::new(client.clone(), recipient);
        let _prefixed = DemoServiceClient::with_prefix(client, "demo", recipient)
            .with_recipient(recipient);
    }

    fn _assert_sum(client: &DemoServiceClient) {
        let _ = async {
            let req = SumRequest { numbers: vec![1, 2, 3] };
            let _result: Result<SumResponse, nats_micro::ClientError<NatsErrorResponse>> =
                client.sum(&req).await;
        };
    }

    fn _assert_profile(client: &DemoServiceClient) {
        let _ = async {
            let _result: Result<UserProfile, nats_micro::ClientError<NatsErrorResponse>> =
                client.get_profile(&"alice".to_string()).await;
        };
    }

    fn _assert_health(client: &DemoServiceClient) {
        let _ = async {
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.health().await;
        };
    }

    fn _assert_echo(client: &DemoServiceClient) {
        let _ = async {
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.echo("hello").await;
        };
    }

    fn _assert_with_variant(client: &DemoServiceClient) {
        let _ = async {
            let req = SumRequest { numbers: vec![1] };
            let opts = nats_micro::ClientCallOptions::new().header("x-trace", "123");
            let _result: Result<SumResponse, nats_micro::ClientError<NatsErrorResponse>> =
                client.sum_with(&req, opts).await;
        };
    }
}

fn main() {
    let def = DemoService::definition();
    assert_eq!(def.metadata.name, "demo_svc");
    assert_eq!(def.endpoints.len(), 4);
}
