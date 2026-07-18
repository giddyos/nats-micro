use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Bytes, BytesMut};
use nats_micro_shared::FrameworkError;
use zeroize::Zeroize;

use super::{
    ENCRYPTED_HEADERS_NAME, EncryptedHeaderOverlay, RESPONSE_PUB_KEY_NAME, SIGNATURE_HEADER_NAME,
    ServiceKeyPair, SignatureTranscript, decode_response_pub_key, decrypt_aead,
    headers::decrypt_header_overlay, verify_signature_for_transcript,
};
use crate::{ErrorReply, Request, Response};

/// Owns the plaintext material for one statically declared encrypted request.
///
/// The decrypted body is held by one zeroizing `BytesMut` and exposed only as
/// a borrowed request view. Decrypted headers live in a zeroizing overlay
/// whose values take precedence over the original NATS headers.
pub struct EncryptedRequest<'a> {
    original: Request<'a>,
    keypair: &'a ServiceKeyPair,
    ephemeral_public_key: [u8; 32],
    plaintext: Option<ZeroizingBytes>,
    overlay: EncryptedHeaderOverlay,
}

impl<'a> EncryptedRequest<'a> {
    pub fn prepare(
        keypair: &'a ServiceKeyPair,
        request: Request<'a>,
        decrypt_payload: bool,
    ) -> Result<Self, ErrorReply> {
        let request_id = request.request_id().existing();
        let headers = request.headers().raw().ok_or_else(|| {
            encryption_error(
                FrameworkError::EncryptRequired,
                format!(
                    "encrypted requests require `{RESPONSE_PUB_KEY_NAME}` and `{SIGNATURE_HEADER_NAME}` headers"
                ),
                request_id,
            )
        })?;
        let ephemeral_public_key = decode_response_pub_key(headers)
            .map_err(|error| {
                encryption_error(
                    FrameworkError::DecryptFailed,
                    format!("failed to decode `{RESPONSE_PUB_KEY_NAME}`: {error}"),
                    request_id,
                )
            })?
            .ok_or_else(|| {
                encryption_error(
                    FrameworkError::EncryptRequired,
                    format!("encrypted requests require header `{RESPONSE_PUB_KEY_NAME}`"),
                    request_id,
                )
            })?;

        let signature = headers
            .get(SIGNATURE_HEADER_NAME)
            .ok_or_else(|| {
                encryption_error(
                    FrameworkError::SignatureMissing,
                    format!("encrypted requests require header `{SIGNATURE_HEADER_NAME}`"),
                    request_id,
                )
            })
            .and_then(|signature| {
                STANDARD.decode(signature.as_str()).map_err(|error| {
                    encryption_error(
                        FrameworkError::SignatureInvalid,
                        format!("failed to decode `{SIGNATURE_HEADER_NAME}`: {error}"),
                        request_id,
                    )
                })
            })?;

        let encryption_key = keypair.derive_encryption_key(&ephemeral_public_key);
        let signature_key = keypair.derive_signature_key(&ephemeral_public_key);
        let encrypted_headers = headers
            .get(ENCRYPTED_HEADERS_NAME)
            .map(crate::NatsHeaderValue::as_str);
        let client_version = headers
            .get("x-client-version")
            .map(crate::NatsHeaderValue::as_str);
        let transcript =
            SignatureTranscript::new(request.subject(), &ephemeral_public_key, request.body())
                .request_id(request_id)
                .client_version(client_version)
                .encrypted_headers_value(encrypted_headers);
        verify_signature_for_transcript(&signature_key, &transcript, &signature).map_err(|_| {
            encryption_error(
                FrameworkError::SignatureInvalid,
                "request signature verification failed; verify the service public key",
                request_id,
            )
        })?;

        let overlay = decrypt_header_overlay(headers, &encryption_key).map_err(|error| {
            encryption_error(
                FrameworkError::DecryptFailed,
                format!("failed to decrypt request headers: {error}"),
                request_id,
            )
        })?;
        if decrypt_payload
            && request.body().get(..super::EPH_PUB_LEN) != Some(ephemeral_public_key.as_slice())
        {
            return Err(encryption_error(
                FrameworkError::DecryptFailed,
                "encrypted headers and payload must use the same ephemeral public key",
                request_id,
            ));
        }
        let plaintext = decrypt_payload
            .then(|| decrypt_payload_once(&encryption_key, request.body(), request_id))
            .transpose()?;

        Ok(Self {
            original: request,
            keypair,
            ephemeral_public_key,
            plaintext,
            overlay,
        })
    }

    #[must_use]
    pub fn request(&self) -> Request<'_> {
        let body = self
            .plaintext
            .as_ref()
            .map_or_else(|| self.original.body(), ZeroizingBytes::as_slice);
        self.original.with_body_and_overlay(body, &self.overlay)
    }

    pub fn encrypt_response(&self, response: Response) -> Result<Response, ErrorReply> {
        let request_id = self.original.request_id().existing();
        let encrypt = |payload: &[u8]| {
            self.keypair
                .encrypt_response(payload, &self.ephemeral_public_key)
                .map(bytes::Bytes::from)
                .map_err(|error| {
                    encryption_error(
                        FrameworkError::EncryptFailed,
                        format!("failed to encrypt response payload: {error}"),
                        request_id,
                    )
                })
        };

        match response {
            Response::Empty => encrypt(&[]).map(Response::Payload),
            Response::Payload(payload) => encrypt(&payload).map(Response::Payload),
            Response::WithHeaders { payload, headers } => {
                encrypt(&payload).map(|payload| Response::WithHeaders { payload, headers })
            }
        }
    }
}

fn decrypt_payload_once(
    encryption_key: &[u8; 32],
    payload: &[u8],
    request_id: Option<&str>,
) -> Result<ZeroizingBytes, ErrorReply> {
    if payload.len() < super::MIN_REQUEST_ENVELOPE {
        return Err(encryption_error(
            FrameworkError::DecryptFailed,
            format!(
                "encrypted payload is too short: expected at least {} bytes, received {}",
                super::MIN_REQUEST_ENVELOPE,
                payload.len()
            ),
            request_id,
        ));
    }
    let nonce: [u8; super::NONCE_LEN] = payload
        [super::EPH_PUB_LEN..super::EPH_PUB_LEN + super::NONCE_LEN]
        .try_into()
        .map_err(|_| {
            encryption_error(
                FrameworkError::DecryptFailed,
                "failed to read the encrypted request nonce",
                request_id,
            )
        })?;
    decrypt_aead(
        "decrypting static request payload",
        encryption_key,
        &nonce,
        &payload[super::EPH_PUB_LEN + super::NONCE_LEN..],
    )
    .map(ZeroizingBytes::from_vec)
    .map_err(|error| {
        encryption_error(
            FrameworkError::DecryptFailed,
            format!("failed to decrypt request payload: {error}"),
            request_id,
        )
    })
}

struct ZeroizingBytes(BytesMut);

impl ZeroizingBytes {
    fn from_vec(plaintext: Vec<u8>) -> Self {
        let plaintext = Bytes::from(plaintext)
            .try_into_mut()
            .expect("newly constructed plaintext bytes are uniquely owned");
        Self(plaintext)
    }

    fn as_slice(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Drop for ZeroizingBytes {
    fn drop(&mut self) {
        self.0.as_mut().zeroize();
    }
}

fn encryption_error(
    kind: FrameworkError,
    message: impl Into<String>,
    request_id: Option<&str>,
) -> ErrorReply {
    ErrorReply::framework(kind, message, request_id)
}
