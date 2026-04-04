use super::{RESPONSE_PUB_KEY_NAME, ServiceKeyPair};
use crate::{
    error::NatsErrorResponse,
    extractors::FromPayload,
    handler::RequestContext,
    response::{IntoNatsResponse, NatsResponse},
};
use nats_micro_shared::FrameworkError;

pub struct Encrypted<T>(pub T);

impl<T> Encrypted<T> {
    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn as_inner(&self) -> &T {
        &self.0
    }

    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: FromPayload> FromPayload for Encrypted<T> {
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let keypair = ctx.states.get::<ServiceKeyPair>().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::NoEncryptionKey,
                "No service encryption key was registered for this endpoint.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let expected_eph_pub = ctx.ephemeral_pub.ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                format!(
                    "This endpoint accepts only encrypted requests. Header `{RESPONSE_PUB_KEY_NAME}` must contain the request's ephemeral public key.",
                ),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        if ctx.request.payload.len() < 32 + 24 + 16 {
            return Err(NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                format!(
                    "encrypted payload is too short: expected at least {} bytes, but received {}",
                    32 + 24 + 16,
                    ctx.request.payload.len()
                ),
            )
            .with_request_id(ctx.request.request_id.clone()));
        }

        let mut eph_pub = [0u8; 32];
        eph_pub.copy_from_slice(&ctx.request.payload[..32]);

        if expected_eph_pub != eph_pub {
            return Err(NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                "The encrypted headers and payload used different ephemeral keys; both must use the same key.",
            )
            .with_request_id(ctx.request.request_id.clone()));
        }

        let shared_key = keypair.derive_shared_key(&eph_pub);

        let plaintext = ServiceKeyPair::decrypt_with_shared_key(&shared_key, &ctx.request.payload)
            .map_err(|error| {
                NatsErrorResponse::framework(
                    FrameworkError::DecryptFailed,
                    format!("failed to decrypt the encrypted request payload: {error}"),
                )
                .with_request_id(ctx.request.request_id.clone())
            })?;

        let mut patched = ctx.clone();
        patched.request.payload = plaintext.into();

        T::from_payload(&patched).map(Encrypted)
    }
}

impl<T: IntoNatsResponse> IntoNatsResponse for Encrypted<T> {
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        let plain = self.0.into_response(ctx)?;

        let eph_pub = ctx.ephemeral_pub.ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::EncryptRequired,
                "Encrypted<T> responses can be returned only for encrypted requests.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let keypair = ctx.states.get::<ServiceKeyPair>().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::NoEncryptionKey,
                "No service encryption key was registered for this endpoint.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let ciphertext = keypair
            .encrypt_response(&plain.payload, &eph_pub)
            .map_err(|error| {
                NatsErrorResponse::framework(
                    FrameworkError::EncryptFailed,
                    format!("failed to encrypt the response payload: {error}"),
                )
                .with_request_id(ctx.request.request_id.clone())
            })?;

        Ok(NatsResponse::new(ciphertext))
    }
}

impl<T> std::ops::Deref for Encrypted<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
