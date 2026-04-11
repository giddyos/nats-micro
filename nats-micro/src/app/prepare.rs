use crate::{error::NatsErrorResponse, request::NatsRequest, state::StateMap};
use nats_micro_shared::FrameworkError;

pub(crate) struct PreparedRequest {
    pub(crate) request: NatsRequest,
    #[cfg(feature = "encryption")]
    pub(crate) ephemeral_pub: Option<[u8; 32]>,
}

pub(super) fn prepare_request_for_dispatch_with_state(
    state: &StateMap,
    mut req: NatsRequest,
) -> Result<PreparedRequest, NatsErrorResponse> {
    #[cfg(not(feature = "encryption"))]
    {
        return Ok(PreparedRequest { request: req });
    }

    #[cfg(feature = "encryption")]
    {
        use crate::encryption::{
            ENCRYPTED_HEADERS_NAME, RESPONSE_PUB_KEY_NAME, SIGNATURE_HEADER_NAME, ServiceKeyPair,
            decode_response_pub_key, decrypt_headers, verify_signature,
        };
        use base64::{Engine, engine::general_purpose::STANDARD};

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

            let shared_key = keypair.derive_shared_key(&eph_pub);

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
            verify_signature(&shared_key, &req.payload, encrypted_headers, &signature).map_err(|_| {
                NatsErrorResponse::framework(
                    FrameworkError::SignatureInvalid,
                    "request signature verification failed; verify that the client used the correct service public key.",
                )
                .with_request_id(req.request_id.clone())
            })?;

            let decrypted_headers = if encrypted_headers.is_some() {
                decrypt_headers(&req.headers, &shared_key).map_err(|error| {
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
