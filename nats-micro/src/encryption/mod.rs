mod headers;
mod payload;

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub(crate) use self::headers::{
    ENCRYPTED_HEADERS_NAME, RESPONSE_PUB_KEY_NAME, SIGNATURE_HEADER_NAME, decode_response_pub_key,
};
pub use self::{headers::decrypt_headers, payload::Encrypted};

type HmacSha256 = Hmac<Sha256>;

const EPH_PUB_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const MIN_REQUEST_ENVELOPE: usize = EPH_PUB_LEN + NONCE_LEN + TAG_LEN;
const MIN_RESPONSE_ENVELOPE: usize = NONCE_LEN + TAG_LEN;

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("failed to decrypt data while {context}")]
    DecryptFailed { context: &'static str },
    #[error(
        "data was too short while {context} (expected at least {expected} bytes, got {actual})"
    )]
    TooShort {
        context: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("failed to encrypt data while {context}")]
    EncryptFailed { context: &'static str },
    #[error("the signature is invalid")]
    SignatureInvalid,
    #[error("no NATS client is configured on ServiceRecipient")]
    NoClient,
    #[error("failed to publish the NATS message: {0}")]
    PublishFailed(String),
    #[error("failed to send the NATS request: {0}")]
    RequestFailed(String),
}

impl EncryptionError {
    pub(crate) fn decrypt_failed(context: &'static str) -> Self {
        Self::DecryptFailed { context }
    }

    pub(crate) fn too_short(context: &'static str, expected: usize, actual: usize) -> Self {
        Self::TooShort {
            context,
            expected,
            actual,
        }
    }

    pub(crate) fn encrypt_failed(context: &'static str) -> Self {
        Self::EncryptFailed { context }
    }
}

fn update_signature_mac(
    mac: &mut HmacSha256,
    payload: &[u8],
    encrypted_headers_value: Option<&str>,
) {
    mac.update(payload);
    if let Some(val) = encrypted_headers_value {
        mac.update(val.as_bytes());
    }
}

fn build_response_envelope(nonce: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    out
}

pub fn compute_signature(
    shared_key: &[u8; 32],
    payload: &[u8],
    encrypted_headers_value: Option<&str>,
) -> Vec<u8> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(shared_key).expect("HMAC accepts any key length");
    update_signature_mac(&mut mac, payload, encrypted_headers_value);
    mac.finalize().into_bytes().to_vec()
}

pub fn verify_signature(
    shared_key: &[u8; 32],
    payload: &[u8],
    encrypted_headers_value: Option<&str>,
    signature: &[u8],
) -> Result<(), EncryptionError> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(shared_key).expect("HMAC accepts any key length");
    update_signature_mac(&mut mac, payload, encrypted_headers_value);
    mac.verify_slice(signature)
        .map_err(|_| EncryptionError::SignatureInvalid)
}

fn encrypt_aead(
    key: &[u8; 32],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; NONCE_LEN]), EncryptionError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|_| {
        EncryptionError::encrypt_failed("encrypting payload with XChaCha20Poly1305")
    })?;
    Ok((ciphertext, nonce_bytes))
}

fn decrypt_aead(
    context: &'static str,
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptionError::decrypt_failed(context))
}

pub struct ServiceKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl ServiceKeyPair {
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn from_base64(encoded: &str) -> Result<Self, base64::DecodeError> {
        let bytes = STANDARD.decode(encoded)?;
        let bytes_len = bytes.len();
        let bytes_array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| base64::DecodeError::InvalidLength(bytes_len))?;
        Ok(Self::from_private_bytes(bytes_array))
    }

    #[cfg(debug_assertions)]
    pub fn expose_secret_b64(&self) -> String {
        STANDARD.encode(self.secret.as_bytes())
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    pub fn public_key_base64(&self) -> String {
        STANDARD.encode(self.public.as_bytes())
    }

    pub fn from_private_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn derive_shared_key(&self, ephemeral_pub_bytes: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let eph_pub = PublicKey::from(*ephemeral_pub_bytes);
        let dh = self.secret.diffie_hellman(&eph_pub);
        Zeroizing::new(*dh.as_bytes())
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if data.len() < MIN_REQUEST_ENVELOPE {
            return Err(EncryptionError::too_short(
                "reading encrypted request envelope",
                MIN_REQUEST_ENVELOPE,
                data.len(),
            ));
        }
        let eph_pub_bytes: [u8; 32] = data[..EPH_PUB_LEN].try_into().map_err(|_| {
            EncryptionError::decrypt_failed("reading ephemeral public key from request payload")
        })?;
        let key = self.derive_shared_key(&eph_pub_bytes);
        let nonce: [u8; NONCE_LEN] = data[EPH_PUB_LEN..EPH_PUB_LEN + NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::decrypt_failed("reading request nonce"))?;
        decrypt_aead(
            "decrypting request payload body",
            &key,
            &nonce,
            &data[EPH_PUB_LEN + NONCE_LEN..],
        )
    }

    pub fn encrypt_response(
        &self,
        plaintext: &[u8],
        ephemeral_pub_bytes: &[u8; 32],
    ) -> Result<Vec<u8>, EncryptionError> {
        let key = self.derive_shared_key(ephemeral_pub_bytes);
        let (ciphertext, nonce) = encrypt_aead(&key, plaintext)?;
        Ok(build_response_envelope(&nonce, &ciphertext))
    }

    pub fn decrypt_with_shared_key(
        key: &[u8; 32],
        data: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        if data.len() < MIN_REQUEST_ENVELOPE {
            return Err(EncryptionError::too_short(
                "reading shared-key request envelope",
                MIN_REQUEST_ENVELOPE,
                data.len(),
            ));
        }
        let nonce: [u8; NONCE_LEN] = data[EPH_PUB_LEN..EPH_PUB_LEN + NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::decrypt_failed("reading shared-key request nonce"))?;
        decrypt_aead(
            "decrypting shared-key request payload body",
            key,
            &nonce,
            &data[EPH_PUB_LEN + NONCE_LEN..],
        )
    }

    pub fn encrypt_response_with_shared_key(
        key: &[u8; 32],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        let (ciphertext, nonce) = encrypt_aead(key, plaintext)?;
        Ok(build_response_envelope(&nonce, &ciphertext))
    }
}

#[derive(Clone)]
pub struct ServiceRecipient {
    public_key: PublicKey,
    client: Option<async_nats::Client>,
}

impl From<[u8; 32]> for ServiceRecipient {
    fn from(bytes: [u8; 32]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl ServiceRecipient {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            public_key: PublicKey::from(bytes),
            client: None,
        }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        *self.public_key.as_bytes()
    }

    pub fn with_client(mut self, client: async_nats::Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn client(&self) -> Option<&async_nats::Client> {
        self.client.as_ref()
    }

    pub fn begin(&self) -> EphemeralContext {
        let eph_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let eph_public = PublicKey::from(&eph_secret);
        let dh = eph_secret.diffie_hellman(&self.public_key);
        let shared_secret = Zeroizing::new(*dh.as_bytes());
        EphemeralContext {
            ephemeral_pub: *eph_public.as_bytes(),
            shared_secret,
        }
    }

    pub fn encrypt(
        &self,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, EphemeralContext), EncryptionError> {
        let ctx = self.begin();
        let encrypted = ctx.encrypt(plaintext)?;
        Ok((encrypted, ctx))
    }

    pub fn request_builder(&self) -> RequestBuilder {
        let ctx = self.begin();
        RequestBuilder {
            ctx,
            client: self.client.clone(),
            plaintext_headers: Vec::new(),
            encrypted_headers: Vec::new(),
            payload: None,
            encrypt_payload: false,
        }
    }
}

pub struct RequestBuilder {
    ctx: EphemeralContext,
    client: Option<async_nats::Client>,
    plaintext_headers: Vec<(String, String)>,
    encrypted_headers: Vec<(String, String)>,
    payload: Option<Vec<u8>>,
    encrypt_payload: bool,
}

impl RequestBuilder {
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.plaintext_headers.push((key.into(), value.into()));
        self
    }

    pub fn encrypted_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.encrypted_headers.push((key.into(), value.into()));
        self
    }

    pub fn bearer_token(self, token: impl Into<String>) -> Self {
        self.encrypted_header("authorization", format!("Bearer {}", token.into()))
    }

    pub fn payload(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.payload = Some(data.into());
        self.encrypt_payload = false;
        self
    }

    pub fn encrypted_payload(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.payload = Some(data.into());
        self.encrypt_payload = true;
        self
    }

    pub fn context(&self) -> &EphemeralContext {
        &self.ctx
    }

    pub fn build(self) -> Result<BuiltRequest, EncryptionError> {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            RESPONSE_PUB_KEY_NAME,
            STANDARD.encode(self.ctx.ephemeral_pub_bytes()),
        );

        for (k, v) in &self.plaintext_headers {
            headers.insert(
                k.as_str(),
                v.parse::<async_nats::HeaderValue>()
                    .unwrap_or_else(|_| async_nats::HeaderValue::from(v.as_str())),
            );
        }

        let encrypted_headers_value = if self.encrypted_headers.is_empty() {
            None
        } else {
            let map: HashMap<&str, &str> = self
                .encrypted_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let json = serde_json::to_vec(&map).map_err(|_| {
                EncryptionError::encrypt_failed("serializing encrypted headers as JSON")
            })?;
            let encrypted = self.ctx.encrypt(&json)?;
            let encoded = STANDARD.encode(&encrypted);
            headers.insert(ENCRYPTED_HEADERS_NAME, encoded.as_str());
            Some(encoded)
        };

        let final_payload = if let Some(data) = &self.payload {
            if self.encrypt_payload {
                self.ctx.encrypt(data)?
            } else {
                data.clone()
            }
        } else {
            Vec::new()
        };

        let signature = compute_signature(
            self.ctx.shared_secret(),
            &final_payload,
            encrypted_headers_value.as_deref(),
        );
        headers.insert(SIGNATURE_HEADER_NAME, STANDARD.encode(&signature));

        Ok(BuiltRequest {
            headers,
            payload: final_payload.into(),
            context: self.ctx,
        })
    }

    pub async fn publish(mut self, subject: impl Into<String>) -> Result<(), EncryptionError> {
        let client = self.client.take().ok_or(EncryptionError::NoClient)?;
        let built = self.build()?;
        client
            .publish_with_headers(subject.into(), built.headers, built.payload)
            .await
            .map_err(|error| EncryptionError::PublishFailed(error.to_string()))
    }

    pub async fn nats_request(
        mut self,
        subject: impl Into<String>,
    ) -> Result<(async_nats::Message, EphemeralContext), EncryptionError> {
        let client = self.client.take().ok_or(EncryptionError::NoClient)?;
        let built = self.build()?;
        let msg = client
            .request_with_headers(subject.into(), built.headers, built.payload)
            .await
            .map_err(|error| EncryptionError::RequestFailed(error.to_string()))?;
        Ok((msg, built.context))
    }
}

pub struct BuiltRequest {
    pub headers: async_nats::HeaderMap,
    pub payload: bytes::Bytes,
    pub context: EphemeralContext,
}

pub struct EphemeralContext {
    ephemeral_pub: [u8; 32],
    shared_secret: Zeroizing<[u8; 32]>,
}

impl EphemeralContext {
    pub fn ephemeral_pub_bytes(&self) -> [u8; 32] {
        self.ephemeral_pub
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let (ciphertext, nonce) = encrypt_aead(&self.shared_secret, plaintext)?;
        let mut out = Vec::with_capacity(EPH_PUB_LEN + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&self.ephemeral_pub);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn decrypt_response(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if data.len() < MIN_RESPONSE_ENVELOPE {
            return Err(EncryptionError::too_short(
                "reading encrypted response envelope",
                MIN_RESPONSE_ENVELOPE,
                data.len(),
            ));
        }
        let nonce: [u8; NONCE_LEN] = data[..NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::decrypt_failed("reading response nonce"))?;
        decrypt_aead(
            "decrypting response payload body",
            &self.shared_secret,
            &nonce,
            &data[NONCE_LEN..],
        )
    }

    pub fn shared_secret(&self) -> &[u8; 32] {
        &self.shared_secret
    }
}
