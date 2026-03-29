use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::encryption::{EncryptionError, ServiceKeyPair};

pub(crate) const ENCRYPTED_HEADERS_NAME: &str = "x-encrypted-headers";
pub(crate) const RESPONSE_PUB_KEY_NAME: &str = "x-ephemeral-pub-key";
pub(crate) const SIGNATURE_HEADER_NAME: &str = "x-signature";

pub(crate) fn decode_response_pub_key(
    headers: &async_nats::HeaderMap,
) -> Result<Option<[u8; 32]>, EncryptionError> {
    let Some(value) = headers.get(RESPONSE_PUB_KEY_NAME) else {
        return Ok(None);
    };

    let decoded = STANDARD.decode(value.as_str().as_bytes()).map_err(|_| {
        EncryptionError::decrypt_failed("decoding x-ephemeral-pub-key header from base64")
    })?;

    decoded
        .try_into()
        .map(Some)
        .map_err(|_| EncryptionError::decrypt_failed("parsing x-ephemeral-pub-key header bytes"))
}

fn decode_header_blob(headers: &async_nats::HeaderMap) -> Result<Option<Vec<u8>>, EncryptionError> {
    let Some(value) = headers.get(ENCRYPTED_HEADERS_NAME) else {
        return Ok(None);
    };

    STANDARD
        .decode(value.as_str().as_bytes())
        .map(Some)
        .map_err(|_| {
            EncryptionError::decrypt_failed("decoding x-encrypted-headers header from base64")
        })
}

pub fn decrypt_headers(
    headers: &async_nats::HeaderMap,
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
