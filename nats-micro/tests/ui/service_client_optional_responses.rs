use nats_micro::{Json, NatsErrorResponse, NatsService, Proto, service, service_handlers};
#[cfg(feature = "encryption")]
use nats_micro::Encrypted;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MaybeJsonResponse {
    value: String,
}

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
struct MaybeProtoResponse {
    #[prost(string, tag = "1")]
    value: String,
}

#[service(name = "optional_response_svc", version = "1.0.0")]
struct OptionalResponseService;

#[service_handlers]
impl OptionalResponseService {
    #[endpoint(subject = "maybe-string")]
    async fn maybe_string() -> Result<Option<String>, NatsErrorResponse> {
        Ok(None)
    }

    #[endpoint(subject = "maybe-json")]
    async fn maybe_json() -> Result<Option<Json<MaybeJsonResponse>>, NatsErrorResponse> {
        Ok(None)
    }

    #[endpoint(subject = "maybe-proto")]
    async fn maybe_proto() -> Result<Option<Proto<MaybeProtoResponse>>, NatsErrorResponse> {
        Ok(None)
    }

    #[cfg(feature = "encryption")]
    #[endpoint(subject = "maybe-encrypted")]
    async fn maybe_encrypted() -> Result<Option<Encrypted<String>>, NatsErrorResponse> {
        Ok(None)
    }
}

#[cfg(feature = "client")]
fn _assert_client_module() {
    use optional_response_service_client::OptionalResponseServiceClient;

    fn _assert_optional_string(client: &OptionalResponseServiceClient) {
        let _ = async {
            let _result: Result<Option<String>, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_string().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<Option<String>, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_string_with(options).await;
        };
    }

    fn _assert_optional_json(client: &OptionalResponseServiceClient) {
        let _ = async {
            let _result: Result<
                Option<MaybeJsonResponse>,
                nats_micro::ClientError<NatsErrorResponse>,
            > = client.maybe_json().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<
                Option<MaybeJsonResponse>,
                nats_micro::ClientError<NatsErrorResponse>,
            > = client.maybe_json_with(options).await;
        };
    }

    fn _assert_optional_proto(client: &OptionalResponseServiceClient) {
        let _ = async {
            let _result: Result<
                Option<MaybeProtoResponse>,
                nats_micro::ClientError<NatsErrorResponse>,
            > = client.maybe_proto().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<
                Option<MaybeProtoResponse>,
                nats_micro::ClientError<NatsErrorResponse>,
            > = client.maybe_proto_with(options).await;
        };
    }

    #[cfg(feature = "encryption")]
    fn _assert_optional_encrypted(client: &OptionalResponseServiceClient) {
        let _ = async {
            let _result: Result<Option<String>, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_encrypted().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<Option<String>, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_encrypted_with(options).await;
        };
    }
}

fn main() {
    let def = OptionalResponseService::definition();
    assert_eq!(def.metadata.name, "optional_response_svc");
    assert!(def.endpoints.len() >= 3);
}