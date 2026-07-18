#![cfg(all(feature = "encryption", feature = "test-util"))]

use nats_micro::{
    ClientTransport, Encrypted, Json, ServiceRecipient, message, service,
    testing::{LiveTestApp, TestApp},
};

#[message]
pub struct SecretInput {
    pub value: String,
}

#[message]
#[derive(Debug, PartialEq, Eq)]
pub struct SecretOutput {
    pub value: String,
    pub protected_header: String,
    pub plain_header: String,
}

#[service(name = "v2-encryption", version = "1.0.0")]
impl EncryptionService {
    #[request("secret")]
    async fn secret(
        input: Encrypted<Json<SecretInput>>,
        #[header("x-protected")] protected_header: &str,
        #[header("x-plain")] plain_header: &str,
    ) -> Encrypted<Json<SecretOutput>> {
        Encrypted(Json(SecretOutput {
            value: input.0.0.value,
            protected_header: protected_header.to_owned(),
            plain_header: plain_header.to_owned(),
        }))
    }

    #[request("response-only")]
    async fn response_only(input: Json<SecretInput>) -> Encrypted<Json<SecretOutput>> {
        Encrypted(Json(SecretOutput {
            value: input.0.value,
            protected_header: "none".to_owned(),
            plain_header: "none".to_owned(),
        }))
    }

    #[request("plain")]
    async fn plain(input: Json<SecretInput>) -> Json<SecretOutput> {
        Json(SecretOutput {
            value: input.0.value,
            protected_header: "plain".to_owned(),
            plain_header: "plain".to_owned(),
        })
    }
}

#[nats_micro::test]
async fn encrypted_generated_client_has_local_full_wire_parity() -> nats_micro::Result<()> {
    let keypair = nats_micro::ServiceKeyPair::generate();
    let app = TestApp::stateless()
        .encryption(keypair)
        .serve(EncryptionService)
        .start();
    let client = app.encrypted_client::<EncryptionService>();

    let response = client
        .secret_call(&SecretInput {
            value: "classified".to_owned(),
        })
        .header("x-protected", "untrusted")?
        .encrypted_header("x-protected", "trusted")?
        .header("x-plain", "visible")?
        .send()
        .await?;
    assert_eq!(
        response,
        SecretOutput {
            value: "classified".to_owned(),
            protected_header: "trusted".to_owned(),
            plain_header: "visible".to_owned(),
        }
    );

    let response_only = client
        .response_only(&SecretInput {
            value: "signed-plaintext".to_owned(),
        })
        .await?;
    assert_eq!(response_only.value, "signed-plaintext");

    let plain = client
        .plain(&SecretInput {
            value: "ordinary".to_owned(),
        })
        .await?;
    assert_eq!(plain.value, "ordinary");

    assert!(EncryptionService::SPEC.operations[0].request_encrypted);
    assert!(EncryptionService::SPEC.operations[0].response_encrypted);
    assert!(!EncryptionService::SPEC.operations[1].request_encrypted);
    assert!(EncryptionService::SPEC.operations[1].response_encrypted);
    assert!(!EncryptionService::SPEC.operations[2].request_encrypted);
    assert!(!EncryptionService::SPEC.operations[2].response_encrypted);
    Ok(())
}

#[nats_micro::test]
async fn signature_tampering_is_rejected_before_handler_dispatch() -> nats_micro::Result<()> {
    let keypair = nats_micro::ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let app = TestApp::stateless()
        .encryption(keypair)
        .serve(EncryptionService)
        .start();
    let transport = app.transport();
    let mut built = recipient
        .request_builder()
        .encrypted_payload(serde_json::to_vec(&SecretInput {
            value: "original".to_owned(),
        })?)
        .build_for_subject("v2-encryption.v1.secret")?;
    built.payload = nats_micro::Bytes::from_static(b"tampered");

    let response = transport
        .request(nats_micro::ClientRequest {
            subject: nats_micro::ClientSubject::Static("v2-encryption.v1.secret"),
            payload: built.payload,
            headers: Some(built.headers),
            timeout: None,
        })
        .await
        .map_err(|error| nats_micro::anyhow::anyhow!("{error:?}"))?;
    assert_eq!(
        response
            .headers
            .as_ref()
            .and_then(|headers| headers.get("Nats-Micro-Error-Kind"))
            .map(nats_micro::NatsHeaderValue::as_str),
        Some("SIGNATURE_INVALID")
    );
    Ok(())
}

#[nats_micro::live_test]
async fn encrypted_generated_client_interoperates_with_live_nats() -> nats_micro::Result<()> {
    let keypair = nats_micro::ServiceKeyPair::generate();
    let live = LiveTestApp::new()
        .encryption(keypair)
        .serve(EncryptionService)
        .start()
        .await?;
    let client = live.encrypted_client::<EncryptionService>();
    let response = client
        .secret_call(&SecretInput {
            value: "live".to_owned(),
        })
        .encrypted_header("x-protected", "live-secret")?
        .header("x-plain", "live-visible")?
        .send()
        .await?;
    assert_eq!(response.value, "live");
    assert_eq!(response.protected_header, "live-secret");
    assert_eq!(response.plain_header, "live-visible");
    live.shutdown().await
}

#[nats_micro::live_test]
async fn encrypted_service_startup_requires_a_key() -> nats_micro::Result<()> {
    match LiveTestApp::new().serve(EncryptionService).start().await {
        Ok(live) => {
            live.shutdown().await?;
            panic!("encrypted service unexpectedly started without a key");
        }
        Err(error) => {
            assert!(error.to_string().contains("no encryption key"));
        }
    }
    Ok(())
}
