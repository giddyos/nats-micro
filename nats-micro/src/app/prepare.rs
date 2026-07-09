#[cfg(feature = "encryption")]
use crate::encryption::{
    ENCRYPTED_HEADERS_NAME, RESPONSE_PUB_KEY_NAME, SIGNATURE_HEADER_NAME, ServiceKeyPair,
    SignatureTranscript, decode_response_pub_key, decrypt_headers, verify_signature_for_transcript,
};
use crate::{error::NatsErrorResponse, request::NatsRequest, state::StateMap};
#[cfg(feature = "encryption")]
use base64::{Engine, engine::general_purpose::STANDARD};
#[cfg(feature = "encryption")]
use nats_micro_shared::FrameworkError;

pub(crate) struct PreparedRequest {
    pub(crate) request: NatsRequest,
    #[cfg(feature = "encryption")]
    pub(crate) ephemeral_pub: Option<[u8; 32]>,
}

#[allow(clippy::too_many_lines)]
pub(super) fn prepare_request_for_dispatch_with_state(
    state: &StateMap,
    req: NatsRequest,
) -> Result<PreparedRequest, NatsErrorResponse> {
    #[cfg(not(feature = "encryption"))]
    {
        let _ = state;
        return Ok(PreparedRequest { request: req });
    }

    #[cfg(feature = "encryption")]
    {
        let mut req = req;

        let response_pub_key = decode_response_pub_key(&req.headers).map_err(|error| {
            NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                format!("failed to decode header `{RESPONSE_PUB_KEY_NAME}`: {error}"),
            )
            .with_request_id(req.request_id.clone())
        })?;

        if let Some(eph_pub) = response_pub_key {
            let keypair = state.get::<ServiceKeyPair>().ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::DecryptFailed,
                    "The service is configured to accept encrypted requests, but no service encryption key was registered.",
                )
                .with_request_id(req.request_id.clone())
            })?;

            let encryption_key = keypair.derive_encryption_key(&eph_pub);
            let signature_key = keypair.derive_signature_key(&eph_pub);

            let sig_header = req.headers.get(SIGNATURE_HEADER_NAME).ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::SignatureMissing,
                    format!(
                        "Header `{SIGNATURE_HEADER_NAME}` is required when `{RESPONSE_PUB_KEY_NAME}` is present."
                    ),
                )
                .with_request_id(req.request_id.clone())
            })?;
            let signature = STANDARD
                .decode(sig_header.as_str().as_bytes())
                .map_err(|error| {
                    NatsErrorResponse::framework(
                        FrameworkError::SignatureInvalid,
                        format!(
                            "failed to decode header `{SIGNATURE_HEADER_NAME}` as base64: {error}"
                        ),
                    )
                    .with_request_id(req.request_id.clone())
                })?;
            let encrypted_headers = req
                .headers
                .get(ENCRYPTED_HEADERS_NAME)
                .map(crate::request::Header::as_str);
            let client_version = req
                .headers
                .get("x-client-version")
                .map(crate::request::Header::as_str);
            let signed_request_id = req
                .headers
                .get("x-request-id")
                .map(crate::request::Header::as_str)
                .filter(|value| *value == req.request_id);
            let transcript = SignatureTranscript::new(&req.subject, &eph_pub, &req.payload)
                .request_id(signed_request_id)
                .client_version(client_version)
                .encrypted_headers_value(encrypted_headers);
            verify_signature_for_transcript(&signature_key, &transcript, &signature).map_err(|_| {
                NatsErrorResponse::framework(
                    FrameworkError::SignatureInvalid,
                    "request signature verification failed; verify that the client used the correct service public key.",
                )
                .with_request_id(req.request_id.clone())
            })?;

            let decrypted_headers = if encrypted_headers.is_some() {
                decrypt_headers(&req.headers, &encryption_key).map_err(|error| {
                    NatsErrorResponse::framework(
                        FrameworkError::DecryptFailed,
                        format!("failed to decrypt the encrypted request headers: {error}"),
                    )
                    .with_request_id(req.request_id.clone())
                })?
            } else {
                std::collections::HashMap::new()
            };

            let mut clean_headers = crate::request::Headers::new();
            for header in &req.headers {
                if header.key.eq_ignore_ascii_case(ENCRYPTED_HEADERS_NAME)
                    || header.key.eq_ignore_ascii_case(RESPONSE_PUB_KEY_NAME)
                    || header.key.eq_ignore_ascii_case(SIGNATURE_HEADER_NAME)
                {
                    continue;
                }
                clean_headers.append(header.key.clone(), header.value.clone());
            }
            req.headers = clean_headers;

            for (key, value) in decrypted_headers {
                req.headers.insert_encrypted(key, value);
            }

            Ok(PreparedRequest {
                request: req,
                ephemeral_pub: Some(eph_pub),
            })
        } else {
            Ok(PreparedRequest {
                request: req,
                ephemeral_pub: None,
            })
        }
    }
}
