use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::encryption::{EncryptionError, EphemeralContext, ServiceKeyPair};

pub(crate) const ENCRYPTED_HEADERS_NAME: &str = "x-encrypted-headers";
pub(crate) const RESPONSE_PUB_KEY_NAME: &str = "x-ephemeral-pub-key";

pub(crate) struct DecryptedHeaders {
    pub headers: HashMap<String, String>,
}

fn encode_pub_key(pub_key: &[u8; 32]) -> String {
    STANDARD.encode(pub_key)
}

pub(crate) fn decode_response_pub_key(
    headers: &async_nats::HeaderMap,
) -> Result<Option<[u8; 32]>, EncryptionError> {
    let Some(value) = headers.get(RESPONSE_PUB_KEY_NAME) else {
        return Ok(None);
    };

    let decoded = STANDARD
        .decode(value.as_str().as_bytes())
        .map_err(|_| EncryptionError::DecryptFailed)?;

    decoded
        .try_into()
        .map(Some)
        .map_err(|_| EncryptionError::DecryptFailed)
}

fn decode_header_blob(headers: &async_nats::HeaderMap) -> Result<Option<Vec<u8>>, EncryptionError> {
    let Some(value) = headers.get(ENCRYPTED_HEADERS_NAME) else {
        return Ok(None);
    };

    STANDARD
        .decode(value.as_str().as_bytes())
        .map(Some)
        .map_err(|_| EncryptionError::DecryptFailed)
}

pub struct EncryptedHeadersBuilder<'a> {
    plaintext_headers: Vec<(String, String)>,
    encrypted_headers: Vec<(String, String)>,
    ctx: &'a EphemeralContext,
}

impl<'a> EncryptedHeadersBuilder<'a> {
    pub fn new(ctx: &'a EphemeralContext) -> Self {
        Self {
            plaintext_headers: Vec::new(),
            encrypted_headers: Vec::new(),
            ctx,
        }
    }

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

    pub fn build(self) -> Result<async_nats::HeaderMap, EncryptionError> {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            RESPONSE_PUB_KEY_NAME,
            encode_pub_key(&self.ctx.ephemeral_pub_bytes()),
        );

        for (k, v) in &self.plaintext_headers {
            headers.insert(
                k.as_str(),
                v.parse::<async_nats::HeaderValue>()
                    .unwrap_or_else(|_| async_nats::HeaderValue::from(v.as_str())),
            );
        }

        if !self.encrypted_headers.is_empty() {
            let map: HashMap<&str, &str> = self
                .encrypted_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            let json = serde_json::to_vec(&map).map_err(|_| EncryptionError::EncryptFailed)?;
            let encrypted = self.ctx.encrypt(&json)?;
            let encoded = STANDARD.encode(&encrypted);
            headers.insert(ENCRYPTED_HEADERS_NAME, encoded.as_str());
        }

        Ok(headers)
    }
}

pub(crate) fn decrypt_request_headers(
    headers: &async_nats::HeaderMap,
    keypair: &ServiceKeyPair,
    response_pub_key: Option<[u8; 32]>,
) -> Result<Option<DecryptedHeaders>, EncryptionError> {
    let Some(decoded) = decode_header_blob(headers)? else {
        return Ok(None);
    };

    let envelope_pub_key: [u8; 32] = decoded
        .get(..32)
        .ok_or(EncryptionError::DecryptFailed)?
        .try_into()
        .map_err(|_| EncryptionError::DecryptFailed)?;
    let response_pub_key = response_pub_key.ok_or(EncryptionError::DecryptFailed)?;
    if response_pub_key != envelope_pub_key {
        return Err(EncryptionError::DecryptFailed);
    }

    let shared_key = keypair.derive_shared_key(&envelope_pub_key);
    let headers =
        decrypt_headers(headers, &shared_key).map_err(|_| EncryptionError::DecryptFailed)?;

    Ok(Some(DecryptedHeaders { headers }))
}

pub fn decrypt_headers(
    headers: &async_nats::HeaderMap,
    shared_key: &[u8; 32],
) -> Result<HashMap<String, String>, EncryptionError> {
    let Some(decoded) = decode_header_blob(headers)? else {
        return Ok(HashMap::new());
    };

    let plaintext = ServiceKeyPair::decrypt_with_shared_key(shared_key, &decoded)
        .map_err(|_| EncryptionError::DecryptFailed)?;
    let map: HashMap<String, String> =
        serde_json::from_slice(&plaintext).map_err(|_| EncryptionError::DecryptFailed)?;
    Ok(map)
}
