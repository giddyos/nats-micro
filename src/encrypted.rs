use crate::{
    encrypted_headers::{RESPONSE_PUB_KEY_NAME, decrypt_headers},
    encryption::ServiceKeyPair,
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
                "no encryption key registered",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let expected_eph_pub = ctx.ephemeral_pub.ok_or_else(|| {
            NatsErrorResponse::framework(FrameworkError::DecryptFailed, format!("this endpoint only accepts encrypted requests with the ephemeral public key present in header '{}'", RESPONSE_PUB_KEY_NAME))
                .with_request_id(ctx.request.request_id.clone())
        })?;

        if ctx.request.payload.len() < 32 + 24 + 16 {
            return Err(NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                "payload too short",
            )
            .with_request_id(ctx.request.request_id.clone()));
        }

        let mut eph_pub = [0u8; 32];
        eph_pub.copy_from_slice(&ctx.request.payload[..32]);

        if expected_eph_pub != eph_pub {
            return Err(NatsErrorResponse::framework(
                FrameworkError::DecryptFailed,
                "multiple ephemeral keys detected; only a single key should be used to encrypt both headers and payload",
            )
            .with_request_id(ctx.request.request_id.clone()));
        }

        let shared_key = keypair.derive_shared_key(&eph_pub);

        let plaintext = ServiceKeyPair::decrypt_with_shared_key(&shared_key, &ctx.request.payload)
            .map_err(|error| {
                NatsErrorResponse::framework(
                    FrameworkError::DecryptFailed,
                    format!("payload decryption failed: {error}"),
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
                "Encrypted<T> response requires the request to have been encrypted",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let keypair = ctx.states.get::<ServiceKeyPair>().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::NoEncryptionKey,
                "no encryption key registered",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let ciphertext = keypair
            .encrypt_response(&plain.payload, &eph_pub)
            .map_err(|_| {
                NatsErrorResponse::framework(
                    FrameworkError::EncryptFailed,
                    "response encryption failed",
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

impl<T> std::ops::DerefMut for Encrypted<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
