#![cfg(all(feature = "napi", feature = "live-test"))]

use nats_micro::{NatsErrorResponse, service, testing::LiveTestApp};

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct NapiLiveInput {
    pub value: String,
}

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct NapiLiveOutput {
    pub value: String,
}

#[service(name = "v6-napi-live", version = "1.0.0", napi)]
impl NapiLiveService {
    #[request("echo.{suffix}")]
    async fn echo(suffix: &str, input: NapiLiveInput) -> NapiLiveOutput {
        NapiLiveOutput {
            value: format!("{}:{suffix}", input.value),
        }
    }

    #[request("fail")]
    async fn fail() -> Result<NapiLiveOutput, NatsErrorResponse> {
        Err(NatsErrorResponse::new(
            409,
            "NAPI_LIVE_CONFLICT",
            "conflict",
            "",
        ))
    }
}

#[nats_micro::live_test]
async fn generated_napi_client_uses_the_live_static_transport() -> nats_micro::Result<()> {
    let live = LiveTestApp::new().serve(NapiLiveService).start().await?;
    let client = JsNapiLiveServiceClient::connect(live.server_url().to_owned()).await?;
    let response = client
        .echo(
            "suffix".to_owned(),
            NapiLiveInput {
                value: "napi".to_owned(),
            },
        )
        .await?;
    assert_eq!(response.value, "napi:suffix");

    let error = client.fail().await.expect_err("typed service failure");
    assert!(error.to_string().contains("NAPI_LIVE_CONFLICT"));
    live.shutdown().await
}
