#![cfg(feature = "encryption")]

use async_nats::HeaderMap;
use nats_micro::{EncryptedHeadersBuilder, EncryptionError, ServiceKeyPair, ServiceRecipient};

#[test]
fn round_trip_encrypt_decrypt_payload() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let plaintext = b"hello, encrypted world!";
    let (encrypted, _eph_ctx) = recipient.encrypt(plaintext).unwrap();

    assert_ne!(&encrypted[..], &plaintext[..]);
    assert!(encrypted.len() > plaintext.len());

    let decrypted = keypair.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn round_trip_response_encryption() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let eph_ctx = recipient.begin();
    let eph_pub = eph_ctx.ephemeral_pub_bytes();

    let response_plaintext = b"response data";
    let encrypted_response = keypair
        .encrypt_response(response_plaintext, &eph_pub)
        .unwrap();

    let decrypted = eph_ctx.decrypt_response(&encrypted_response).unwrap();
    assert_eq!(decrypted, response_plaintext);
}

#[test]
fn headers_and_payload_share_ephemeral_key() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let eph_ctx = recipient.begin();

    let payload = b"payload data";
    let encrypted_payload = eph_ctx.encrypt(payload).unwrap();

    let headers = EncryptedHeadersBuilder::new(&eph_ctx)
        .header("x-request-id", "req-123")
        .encrypted_header("authorization", "Bearer secret-token")
        .build()
        .unwrap();

    let eph_from_payload: [u8; 32] = encrypted_payload[..32].try_into().unwrap();
    assert_eq!(eph_from_payload, eph_ctx.ephemeral_pub_bytes());

    let decrypted_payload = keypair.decrypt(&encrypted_payload).unwrap();
    assert_eq!(decrypted_payload, payload);

    let shared_key = keypair.derive_shared_key(&eph_from_payload);
    let decrypted_headers = nats_micro::encrypted_headers_decrypt(&headers, &shared_key).unwrap();
    assert_eq!(
        decrypted_headers.get("authorization").unwrap(),
        "Bearer secret-token"
    );
}

#[test]
fn response_uses_same_shared_secret_as_request() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let eph_ctx = recipient.begin();
    let encrypted_payload = eph_ctx.encrypt(b"request").unwrap();

    let eph_pub: [u8; 32] = encrypted_payload[..32].try_into().unwrap();
    let response_plaintext = b"response from server";
    let encrypted_response = keypair
        .encrypt_response(response_plaintext, &eph_pub)
        .unwrap();

    let decrypted = eph_ctx.decrypt_response(&encrypted_response).unwrap();
    assert_eq!(decrypted, response_plaintext);
}

#[test]
fn wrong_key_fails_decryption() {
    let keypair1 = ServiceKeyPair::generate();
    let keypair2 = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair1.public_key_bytes());

    let (encrypted, _) = recipient.encrypt(b"secret").unwrap();

    let result = keypair2.decrypt(&encrypted);
    assert!(result.is_err());
}

#[test]
fn truncated_ciphertext_fails() {
    let keypair = ServiceKeyPair::generate();
    let result = keypair.decrypt(&[0u8; 10]);
    assert!(matches!(result, Err(EncryptionError::TooShort)));
}

#[test]
fn from_private_bytes_round_trip() {
    let private_bytes: [u8; 32] = {
        let mut buf = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut buf);
        buf
    };

    let keypair1 = ServiceKeyPair::from_private_bytes(private_bytes);
    let pub_bytes = keypair1.public_key_bytes();

    let recipient = ServiceRecipient::from_bytes(pub_bytes);
    let (encrypted, eph_ctx) = recipient.encrypt(b"persistence test").unwrap();

    let keypair2 = ServiceKeyPair::from_private_bytes(private_bytes);
    let decrypted = keypair2.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, b"persistence test");

    let response_enc = keypair2
        .encrypt_response(b"response", &eph_ctx.ephemeral_pub_bytes())
        .unwrap();
    let response_dec = eph_ctx.decrypt_response(&response_enc).unwrap();
    assert_eq!(response_dec, b"response");
}

#[test]
fn encrypted_headers_builder_plaintext_only() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let eph = recipient.begin();

    let headers = EncryptedHeadersBuilder::new(&eph)
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .build()
        .unwrap();

    assert!(headers.get("x-encrypted-headers").is_none());
    assert!(headers.get("x-ephemeral-pub-key").is_some());
    assert_eq!(headers.get("x-request-id").unwrap().as_str(), "req-1");
}

#[test]
fn encrypted_headers_builder_mixed() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let eph = recipient.begin();

    let headers = EncryptedHeadersBuilder::new(&eph)
        .header("x-request-id", "req-2")
        .encrypted_header("x-user-id", "user-42")
        .bearer_token("my-token")
        .build()
        .unwrap();

    assert_eq!(headers.get("x-request-id").unwrap().as_str(), "req-2");
    assert!(headers.get("x-encrypted-headers").is_some());
    assert!(headers.get("x-ephemeral-pub-key").is_some());
    assert!(headers.get("authorization").is_none());

    let shared_key = keypair.derive_shared_key(&eph.ephemeral_pub_bytes());
    let decrypted = nats_micro::encrypted_headers_decrypt(&headers, &shared_key).unwrap();
    assert_eq!(decrypted.get("x-user-id").unwrap(), "user-42");
    assert_eq!(decrypted.get("authorization").unwrap(), "Bearer my-token");
}

#[test]
fn empty_payload_encrypt_decrypt() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let (encrypted, _) = recipient.encrypt(b"").unwrap();
    let decrypted = keypair.decrypt(&encrypted).unwrap();
    assert!(decrypted.is_empty());
}

#[test]
fn large_payload_encrypt_decrypt() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let large = vec![0xABu8; 1_000_000];
    let (encrypted, _) = recipient.encrypt(&large).unwrap();
    let decrypted = keypair.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, large);
}

#[test]
fn tampered_ciphertext_fails() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let (mut encrypted, _) = recipient.encrypt(b"tamper test").unwrap();
    let last = encrypted.len() - 1;
    encrypted[last] ^= 0xFF;

    assert!(keypair.decrypt(&encrypted).is_err());
}

#[test]
fn no_encrypted_headers_returns_empty_map() {
    let headers = HeaderMap::new();
    let dummy_key = [0u8; 32];
    let result = nats_micro::encrypted_headers_decrypt(&headers, &dummy_key).unwrap();
    assert!(result.is_empty());
}

#[test]
fn full_encrypted_request_response_cycle() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let eph = recipient.begin();
    let request_payload = b"full cycle request data";
    let encrypted_request = eph.encrypt(request_payload).unwrap();

    let headers = EncryptedHeadersBuilder::new(&eph)
        .header("x-request-id", "cycle-1")
        .encrypted_header("x-secret", "classified")
        .build()
        .unwrap();

    let eph_pub: [u8; 32] = encrypted_request[..32].try_into().unwrap();
    let shared_key = keypair.derive_shared_key(&eph_pub);

    let decrypted_request = nats_micro::encryption::ServiceKeyPair::decrypt_with_shared_key(
        &shared_key,
        &encrypted_request,
    )
    .unwrap();
    assert_eq!(decrypted_request, request_payload);

    let decrypted_headers = nats_micro::encrypted_headers_decrypt(&headers, &shared_key).unwrap();
    assert_eq!(decrypted_headers.get("x-secret").unwrap(), "classified");

    let response_payload = b"full cycle response";
    let encrypted_response = keypair
        .encrypt_response(response_payload, &eph_pub)
        .unwrap();

    let decrypted_response = eph.decrypt_response(&encrypted_response).unwrap();
    assert_eq!(decrypted_response, response_payload);
}

#[test]
fn each_encryption_produces_different_ciphertext() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let plaintext = b"nonce uniqueness test";
    let (enc1, _) = recipient.encrypt(plaintext).unwrap();
    let (enc2, _) = recipient.encrypt(plaintext).unwrap();

    assert_ne!(enc1, enc2);

    assert_eq!(keypair.decrypt(&enc1).unwrap(), plaintext);
    assert_eq!(keypair.decrypt(&enc2).unwrap(), plaintext);
}
