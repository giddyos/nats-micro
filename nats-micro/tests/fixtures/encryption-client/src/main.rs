#![allow(clippy::unused_async)]

use nats_micro::{
    ClientCallOptions, Encrypted, Json, NatsErrorResponse, Payload, ServiceKeyPair,
    ServiceRecipient, service, service_handlers,
};

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct SecretRequest {
    pub value: String,
}

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct SecretResponse {
    pub value: String,
}

#[service(name = "encrypted-service", version = "1.0.0")]
pub struct EncryptedService;

#[service_handlers]
impl EncryptedService {
    #[endpoint(subject = "secret", group = "secure")]
    async fn secret(
        payload: Payload<Encrypted<Json<SecretRequest>>>,
    ) -> Result<Encrypted<Json<SecretResponse>>, NatsErrorResponse> {
        Ok(Encrypted(Json(SecretResponse {
            value: payload.into_inner().value,
        })))
    }

    #[endpoint(subject = "secret-string", group = "secure")]
    async fn secret_string(
        payload: Payload<Encrypted<String>>,
    ) -> Result<Encrypted<String>, NatsErrorResponse> {
        Ok(payload.into_wrapped())
    }
}

fn uses_encrypted_client(client: nats_micro::NatsClient) {
    let keypair = ServiceKeyPair::generate();
    let pubkey = keypair.public_key_bytes();
    let recipient = ServiceRecipient::from_bytes(pubkey);
    assert_eq!(recipient.to_bytes(), pubkey);

    let generated = encrypted_service_client::EncryptedServiceClient::new(
        client.clone(),
        Some(pubkey),
    );
    let _prefixed = encrypted_service_client::EncryptedServiceClient::with_prefix(
        client.clone(),
        "tenant-a",
        Some(pubkey),
    );
    let _built = encrypted_service_client::EncryptedServiceClient::builder(client)
        .prefix("tenant-a")
        .with_recipient(pubkey)
        .default_encrypted_header("x-secure", "fixture")
        .try_default_encrypted_header("x-tenant-id", "tenant-a")
        .expect("valid encrypted header")
        .default_bearer_token("default-token")
        .build();

    let request = SecretRequest {
        value: "secret".to_string(),
    };

    let _ = async move {
        let _json: Result<SecretResponse, nats_micro::ClientError<NatsErrorResponse>> =
            generated.secret(&request).await;
        let _json_with = generated
            .secret_with(
                &request,
                ClientCallOptions::new()
                    .encrypted_header("x-request-secret", "secret")
                    .bearer_token("call-token"),
            )
            .await;
        let _raw: Result<String, nats_micro::ClientError<NatsErrorResponse>> =
            generated.secret_string("secret").await;
    };
}

fn main() {
    let _ = uses_encrypted_client;
}
