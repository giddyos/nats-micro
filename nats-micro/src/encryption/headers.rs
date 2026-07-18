use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use zeroize::Zeroizing;

use super::{EncryptionError, ServiceKeyPair};

pub(crate) const ENCRYPTED_HEADERS_NAME: &str = "x-encrypted-headers";
pub(crate) const RESPONSE_PUB_KEY_NAME: &str = "x-ephemeral-pub-key";
pub(crate) const SIGNATURE_HEADER_NAME: &str = "x-signature";

#[derive(Debug, Default)]
pub(crate) struct EncryptedHeaderOverlay {
    entries: Vec<(Zeroizing<String>, Zeroizing<String>)>,
}

impl EncryptedHeaderOverlay {
    #[must_use]
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rfind(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }
}

pub(crate) fn is_reserved_encryption_header_name(key: &str) -> bool {
    key.eq_ignore_ascii_case(ENCRYPTED_HEADERS_NAME)
        || key.eq_ignore_ascii_case(RESPONSE_PUB_KEY_NAME)
        || key.eq_ignore_ascii_case(SIGNATURE_HEADER_NAME)
}

#[doc(hidden)]
pub trait HeaderLookup {
    fn get_str(&self, key: &str) -> Option<&str>;
}

impl HeaderLookup for crate::NatsHeaderMap {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).map(crate::NatsHeaderValue::as_str)
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
    encryption_key: &[u8; 32],
) -> Result<HashMap<String, String>, EncryptionError> {
    let Some(decoded) = decode_header_blob(headers)? else {
        return Ok(HashMap::new());
    };

    let plaintext = ServiceKeyPair::decrypt_with_encryption_key(encryption_key, &decoded)
        .map_err(|_| EncryptionError::decrypt_failed("decrypting encrypted headers payload"))?;
    let map: HashMap<String, String> = serde_json::from_slice(&plaintext)
        .map_err(|_| EncryptionError::decrypt_failed("deserializing decrypted headers JSON"))?;
    if let Some(key) = map
        .keys()
        .find(|key| is_reserved_encryption_header_name(key.as_str()))
    {
        return Err(EncryptionError::reserved_header(key));
    }
    Ok(map)
}

pub(crate) fn decrypt_header_overlay<H: HeaderLookup>(
    headers: &H,
    encryption_key: &[u8; 32],
) -> Result<EncryptedHeaderOverlay, EncryptionError> {
    let Some(decoded) = decode_header_blob(headers)? else {
        return Ok(EncryptedHeaderOverlay::default());
    };

    let plaintext = Zeroizing::new(
        ServiceKeyPair::decrypt_with_encryption_key(encryption_key, &decoded)
            .map_err(|_| EncryptionError::decrypt_failed("decrypting encrypted headers payload"))?,
    );
    let map: HashMap<String, String> = serde_json::from_slice(&plaintext)
        .map_err(|_| EncryptionError::decrypt_failed("deserializing decrypted headers JSON"))?;
    if let Some(key) = map
        .keys()
        .find(|key| is_reserved_encryption_header_name(key.as_str()))
    {
        return Err(EncryptionError::reserved_header(key));
    }

    Ok(EncryptedHeaderOverlay {
        entries: map
            .into_iter()
            .map(|(name, value)| (Zeroizing::new(name), Zeroizing::new(value)))
            .collect(),
    })
}
