use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const HKDF_INFO: &[u8] = b"nats-micro-v1";
const EPH_PUB_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const MIN_REQUEST_ENVELOPE: usize = EPH_PUB_LEN + NONCE_LEN + TAG_LEN;
const MIN_RESPONSE_ENVELOPE: usize = NONCE_LEN + TAG_LEN;

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("decryption failed")]
    DecryptFailed,
    #[error("ciphertext too short")]
    TooShort,
    #[error("encryption failed")]
    EncryptFailed,
}

fn derive_key(dh_output: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, dh_output);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .expect("32-byte output is always valid for HKDF-SHA256");
    Zeroizing::new(key)
}

fn encrypt_aead(
    key: &[u8; 32],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; NONCE_LEN]), EncryptionError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| EncryptionError::EncryptFailed)?;
    Ok((ciphertext, nonce_bytes))
}

fn decrypt_aead(
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptionError::DecryptFailed)
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

    pub fn public_key_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    pub fn from_private_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn derive_shared_key(&self, ephemeral_pub_bytes: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let eph_pub = PublicKey::from(*ephemeral_pub_bytes);
        let dh = self.secret.diffie_hellman(&eph_pub);
        derive_key(dh.as_bytes())
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if data.len() < MIN_REQUEST_ENVELOPE {
            return Err(EncryptionError::TooShort);
        }
        let eph_pub_bytes: [u8; 32] = data[..EPH_PUB_LEN]
            .try_into()
            .map_err(|_| EncryptionError::DecryptFailed)?;
        let key = self.derive_shared_key(&eph_pub_bytes);
        let nonce: [u8; NONCE_LEN] = data[EPH_PUB_LEN..EPH_PUB_LEN + NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::DecryptFailed)?;
        decrypt_aead(&key, &nonce, &data[EPH_PUB_LEN + NONCE_LEN..])
    }

    pub fn encrypt_response(
        &self,
        plaintext: &[u8],
        ephemeral_pub_bytes: &[u8; 32],
    ) -> Result<Vec<u8>, EncryptionError> {
        let key = self.derive_shared_key(ephemeral_pub_bytes);
        let (ciphertext, nonce) = encrypt_aead(&key, plaintext)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn decrypt_with_shared_key(
        key: &[u8; 32],
        data: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        if data.len() < MIN_REQUEST_ENVELOPE {
            return Err(EncryptionError::TooShort);
        }
        let nonce: [u8; NONCE_LEN] = data[EPH_PUB_LEN..EPH_PUB_LEN + NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::DecryptFailed)?;
        decrypt_aead(key, &nonce, &data[EPH_PUB_LEN + NONCE_LEN..])
    }

    pub fn encrypt_response_with_shared_key(
        key: &[u8; 32],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        let (ciphertext, nonce) = encrypt_aead(key, plaintext)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }
}

pub struct ServiceRecipient {
    public_key: PublicKey,
}

impl ServiceRecipient {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            public_key: PublicKey::from(bytes),
        }
    }

    pub fn begin(&self) -> EphemeralContext {
        let eph_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let eph_public = PublicKey::from(&eph_secret);
        let dh = eph_secret.diffie_hellman(&self.public_key);
        let shared_secret = derive_key(dh.as_bytes());
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
            return Err(EncryptionError::TooShort);
        }
        let nonce: [u8; NONCE_LEN] = data[..NONCE_LEN]
            .try_into()
            .map_err(|_| EncryptionError::DecryptFailed)?;
        decrypt_aead(&self.shared_secret, &nonce, &data[NONCE_LEN..])
    }

    pub fn shared_secret(&self) -> &[u8; 32] {
        &self.shared_secret
    }
}
