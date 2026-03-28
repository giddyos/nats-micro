use crate::{
    encrypted_headers::decrypt_headers,
    encryption::ServiceKeyPair,
    error::NatsErrorResponse,
    extractors::FromRequest,
    handler::RequestContext,
    response::{IntoNatsResponse, NatsResponse},
};

pub struct Encrypted<T>(pub T);

impl<T: FromRequest> FromRequest for Encrypted<T> {
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let keypair = ctx.states.get::<ServiceKeyPair>().ok_or_else(|| {
            NatsErrorResponse::internal("NO_ENCRYPTION_KEY", "no encryption key registered")
                .with_request_id(ctx.request.request_id.clone())
        })?;

        let expected_eph_pub = ctx.ephemeral_pub.ok_or_else(|| {
            NatsErrorResponse::bad_request("DECRYPT_FAILED", "payload decryption failed")
                .with_request_id(ctx.request.request_id.clone())
        })?;

        if ctx.request.payload.len() < 32 + 24 + 16 {
            return Err(
                NatsErrorResponse::bad_request("DECRYPT_FAILED", "payload too short")
                    .with_request_id(ctx.request.request_id.clone()),
            );
        }

        let mut eph_pub = [0u8; 32];
        eph_pub.copy_from_slice(&ctx.request.payload[..32]);

        if expected_eph_pub != eph_pub {
            return Err(NatsErrorResponse::bad_request(
                "DECRYPT_FAILED",
                "multiple ephemeral keys detected; only a single key should be used to encrypt both headers and payload",
            )
            .with_request_id(ctx.request.request_id.clone()));
        }

        let shared_key = keypair.derive_shared_key(&eph_pub);

        let plaintext = ServiceKeyPair::decrypt_with_shared_key(&shared_key, &ctx.request.payload)
            .map_err(|_| {
                NatsErrorResponse::bad_request("DECRYPT_FAILED", "payload decryption failed")
                    .with_request_id(ctx.request.request_id.clone())
            })?;

        let recovered_headers =
            decrypt_headers(&ctx.request.headers, &shared_key).map_err(|_| {
                NatsErrorResponse::bad_request("DECRYPT_FAILED", "header decryption failed")
                    .with_request_id(ctx.request.request_id.clone())
            })?;

        let mut patched = ctx.clone();
        patched.request.payload = plaintext.into();
        for (k, v) in recovered_headers {
            if let Ok(val) = v.parse::<async_nats::HeaderValue>() {
                patched.request.headers.insert(k.as_str(), val);
            }
        }

        T::from_request(&patched).map(Encrypted)
    }
}

impl<T: IntoNatsResponse> IntoNatsResponse for Encrypted<T> {
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        let plain = self.0.into_response(ctx)?;

        let eph_pub = ctx.ephemeral_pub.ok_or_else(|| {
            NatsErrorResponse::bad_request(
                "ENCRYPT_REQUIRED",
                "Encrypted<T> response requires the request to have been encrypted",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let keypair = ctx.states.get::<ServiceKeyPair>().ok_or_else(|| {
            NatsErrorResponse::internal("NO_ENCRYPTION_KEY", "no encryption key registered")
                .with_request_id(ctx.request.request_id.clone())
        })?;

        let ciphertext = keypair
            .encrypt_response(&plain.payload, &eph_pub)
            .map_err(|_| {
                NatsErrorResponse::internal("ENCRYPT_FAILED", "response encryption failed")
                    .with_request_id(ctx.request.request_id.clone())
            })?;

        Ok(NatsResponse {
            payload: ciphertext.into(),
        })
    }
}
