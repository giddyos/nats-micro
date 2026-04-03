use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{EncryptionError, ServiceKeyPair};
use crate::request::Headers;

pub(crate) const ENCRYPTED_HEADERS_NAME: &str = "x-encrypted-headers";
pub(crate) const RESPONSE_PUB_KEY_NAME: &str = "x-ephemeral-pub-key";
pub(crate) const SIGNATURE_HEADER_NAME: &str = "x-signature";

#[doc(hidden)]
pub trait HeaderLookup {
    fn get_str(&self, key: &str) -> Option<&str>;
}

impl HeaderLookup for Headers {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).map(crate::request::Header::as_str)
    }
}

impl HeaderLookup for async_nats::HeaderMap {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).map(async_nats::HeaderValue::as_str)
    }
}

pub(crate) fn decode_response_pub_key<H: HeaderLookup>(
    headers: &H,
) -> Result<Option<[u8; 32]>, EncryptionError> {
    let Some(value) = headers.get_str(RESPONSE_PUB_KEY_NAME) else {
        return Ok(None);
    };

    let decoded = STANDARD.decode(value.as_bytes()).map_err(|_| {
        EncryptionError::decrypt_failed("decoding x-ephemeral-pub-key header from base64")
    })?;

    decoded
        .try_into()
        .map(Some)
        .map_err(|_| EncryptionError::decrypt_failed("parsing x-ephemeral-pub-key header bytes"))
}

fn decode_header_blob<H: HeaderLookup>(headers: &H) -> Result<Option<Vec<u8>>, EncryptionError> {
    let Some(value) = headers.get_str(ENCRYPTED_HEADERS_NAME) else {
        return Ok(None);
    };

    STANDARD.decode(value.as_bytes()).map(Some).map_err(|_| {
        EncryptionError::decrypt_failed("decoding x-encrypted-headers header from base64")
    })
}

pub fn decrypt_headers<H: HeaderLookup>(
    headers: &H,
    shared_key: &[u8; 32],
) -> Result<HashMap<String, String>, EncryptionError> {
    let Some(decoded) = decode_header_blob(headers)? else {
        return Ok(HashMap::new());
    };

    let plaintext = ServiceKeyPair::decrypt_with_shared_key(shared_key, &decoded)
        .map_err(|_| EncryptionError::decrypt_failed("decrypting encrypted headers payload"))?;
    let map: HashMap<String, String> = serde_json::from_slice(&plaintext)
        .map_err(|_| EncryptionError::decrypt_failed("deserializing decrypted headers JSON"))?;
    Ok(map)
}
