use nats_micro::{
    Json, NatsErrorResponse, NatsService, Payload, Proto, service, service_handlers,
};
#[cfg(feature = "encryption")]
use nats_micro::Encrypted;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct MaybeJsonRequest {
    value: String,
}

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
struct MaybeProtoRequest {
    #[prost(string, tag = "1")]
    value: String,
}

#[service(name = "optional_svc", version = "1.0.0")]
struct OptionalService;

#[service_handlers]
impl OptionalService {
    #[endpoint(subject = "maybe-json")]
    async fn maybe_json(
        payload: Payload<Option<Json<MaybeJsonRequest>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .0
            .map(|payload| payload.0.value)
            .unwrap_or_default())
    }

    #[endpoint(subject = "maybe-proto")]
    async fn maybe_proto(
        payload: Payload<Option<Proto<MaybeProtoRequest>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .0
            .map(|payload| payload.0.value)
            .unwrap_or_default())
    }

    #[cfg(feature = "encryption")]
    #[endpoint(subject = "maybe-encrypted")]
    async fn maybe_encrypted(
        payload: Payload<Option<Encrypted<String>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload.0.map(|payload| payload.0).unwrap_or_default())
    }
}

#[cfg(feature = "client")]
fn _assert_client_module() {
    use optional_service_client::OptionalServiceClient;

    fn _assert_optional_json(client: &OptionalServiceClient) {
        let _ = async {
            let request = MaybeJsonRequest {
                value: "hello".to_string(),
            };
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_json(Some(&request)).await;
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_json(None).await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_json_with(Some(&request), options).await;
        };
    }

    fn _assert_optional_proto(client: &OptionalServiceClient) {
        let _ = async {
            let request = MaybeProtoRequest {
                value: "hello".to_string(),
            };
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_proto(Some(&request)).await;
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_proto(None).await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_proto_with(Some(&request), options).await;
        };
    }

    #[cfg(feature = "encryption")]
    fn _assert_optional_encrypted(client: &OptionalServiceClient) {
        let _ = async {
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_encrypted(Some("secret")).await;
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_encrypted(None).await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
                client.maybe_encrypted_with(Some("secret"), options).await;
        };
    }
}

fn main() {
    let def = OptionalService::definition();
    assert_eq!(def.metadata.name, "optional_svc");
    assert!(def.endpoints.len() >= 2);
}