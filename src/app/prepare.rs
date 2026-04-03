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
        use crate::{
            encrypted_headers::{
                ENCRYPTED_HEADERS_NAME, RESPONSE_PUB_KEY_NAME, SIGNATURE_HEADER_NAME,
                decode_response_pub_key, decrypt_headers,
            },
            encryption::{ServiceKeyPair, verify_signature},
        };
        use base64::{Engine, engine::general_purpose::STANDARD};

        let response_pub_key = decode_response_pub_key(&req.headers).map_err(|error| {
            NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                format!("failed to decode ephemeral public key from headers: {error}"),
            )
            .with_request_id(req.request_id.clone())
        })?;

        if let Some(eph_pub) = &response_pub_key {
            let keypair = state.get::<ServiceKeyPair>().ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::DecryptFailed,
                    "service encryption key not configured",
                )
                .with_request_id(req.request_id.clone())
            })?;

            println!(
                "Derived shared key for ephemeral public key: {}",
                STANDARD.encode(eph_pub)
            );

            let shared_key = keypair.derive_shared_key(eph_pub);

            let sig_header = req.headers.get(SIGNATURE_HEADER_NAME).ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::SignatureMissing,
                    "x-signature header is required when x-ephemeral-pub-key is present",
                )
                .with_request_id(req.request_id.clone())
            })?;
            let signature = STANDARD
                .decode(sig_header.as_str().as_bytes())
                .map_err(|_| {
                    NatsErrorResponse::framework(
                        FrameworkError::SignatureInvalid,
                        "invalid signature encoding",
                    )
                    .with_request_id(req.request_id.clone())
                })?;
            let enc_hdr_val = req.headers.get(ENCRYPTED_HEADERS_NAME).map(|v| v.as_str());
            verify_signature(&shared_key, &req.payload, enc_hdr_val, &signature).map_err(|_| {
                NatsErrorResponse::framework(
                    FrameworkError::SignatureInvalid,
                    "Request signature verification failed. Please ensure that the service public key is correct.",
                )
                .with_request_id(req.request_id.clone())
            })?;

            let decrypted_headers = if req.headers.get(ENCRYPTED_HEADERS_NAME).is_some() {
                decrypt_headers(&req.headers, &shared_key).map_err(|error| {
                    NatsErrorResponse::framework(
                        FrameworkError::DecryptFailed,
                        format!("failed to decrypt the request headers: {error}"),
                    )
                    .with_request_id(req.request_id.clone())
                })?
            } else {
                std::collections::HashMap::new()
            };

            let mut clean_headers = crate::request::Headers::new();
            for header in req.headers.iter() {
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
                ephemeral_pub: response_pub_key,
            })
        } else {
            Ok(PreparedRequest {
                request: req,
                ephemeral_pub: None,
            })
        }
    }
}
