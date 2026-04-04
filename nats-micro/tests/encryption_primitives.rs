#![cfg(feature = "encryption")]
#![allow(clippy::redundant_closure_for_method_calls)]

use async_nats::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD};
use nats_micro::{
    EncryptionError, ServiceKeyPair, ServiceRecipient, encrypted_headers_decrypt,
    encryption::{compute_signature, verify_signature},
};

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

    let built = recipient
        .request_builder()
        .header("x-request-id", "req-123")
        .encrypted_header("authorization", "Bearer secret-token")
        .encrypted_payload(b"payload data".to_vec())
        .build()
        .unwrap();

    let eph_from_payload: [u8; 32] = built.payload[..32].try_into().unwrap();
    assert_eq!(eph_from_payload, built.context.ephemeral_pub_bytes());

    let decrypted_payload = keypair.decrypt(&built.payload).unwrap();
    assert_eq!(decrypted_payload, b"payload data");

    let shared_key = keypair.derive_shared_key(&eph_from_payload);
    let decrypted_headers = encrypted_headers_decrypt(&built.headers, &shared_key).unwrap();
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
    let error = result.expect_err("wrong key should fail");
    assert!(
        error
            .to_string()
            .contains("decrypting request payload body")
    );
}

#[test]
fn truncated_ciphertext_fails() {
    let keypair = ServiceKeyPair::generate();
    let result = keypair.decrypt(&[0u8; 10]);
    assert!(matches!(result, Err(EncryptionError::TooShort { .. })));
}

#[test]
fn invalid_encrypted_headers_report_context() {
    let mut headers = HeaderMap::new();
    headers.insert("x-encrypted-headers", "not-base64");

    let error = encrypted_headers_decrypt(&headers, &[0u8; 32]).expect_err("invalid header blob");
    assert!(
        error
            .to_string()
            .contains("decoding x-encrypted-headers header from base64")
    );
}

#[test]
fn invalid_encrypted_headers_json_reports_context() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let eph_ctx = recipient.begin();
    let encrypted_blob = eph_ctx.encrypt(b"not-json").unwrap();
    let shared_key = keypair.derive_shared_key(&eph_ctx.ephemeral_pub_bytes());

    let mut headers = HeaderMap::new();
    headers.insert("x-encrypted-headers", STANDARD.encode(&encrypted_blob));

    let error = encrypted_headers_decrypt(&headers, &shared_key)
        .expect_err("non-json encrypted headers should fail");
    assert!(
        error
            .to_string()
            .contains("deserializing decrypted headers JSON")
    );
}

#[test]
fn tampered_response_reports_response_decrypt_context() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let eph_ctx = recipient.begin();
    let eph_pub = eph_ctx.ephemeral_pub_bytes();

    let mut encrypted_response = keypair
        .encrypt_response(b"response data", &eph_pub)
        .unwrap();
    let last = encrypted_response.len() - 1;
    encrypted_response[last] ^= 0xFF;

    let error = eph_ctx
        .decrypt_response(&encrypted_response)
        .expect_err("tampered response should fail");
    assert!(
        error
            .to_string()
            .contains("decrypting response payload body")
    );
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
fn request_builder_plaintext_only() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let built = recipient
        .request_builder()
        .header("x-request-id", "req-1")
        .header("content-type", "application/json")
        .build()
        .unwrap();

    assert!(built.headers.get("x-encrypted-headers").is_none());
    assert!(built.headers.get("x-ephemeral-pub-key").is_some());
    assert!(built.headers.get("x-signature").is_some());
    assert_eq!(built.headers.get("x-request-id").unwrap().as_str(), "req-1");
}

#[test]
fn request_builder_mixed() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let built = recipient
        .request_builder()
        .header("x-request-id", "req-2")
        .encrypted_header("x-user-id", "user-42")
        .bearer_token("my-token")
        .build()
        .unwrap();

    assert_eq!(built.headers.get("x-request-id").unwrap().as_str(), "req-2");
    assert!(built.headers.get("x-encrypted-headers").is_some());
    assert!(built.headers.get("x-ephemeral-pub-key").is_some());
    assert!(built.headers.get("x-signature").is_some());
    assert!(built.headers.get("authorization").is_none());

    let shared_key = keypair.derive_shared_key(&built.context.ephemeral_pub_bytes());
    let decrypted = encrypted_headers_decrypt(&built.headers, &shared_key).unwrap();
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
    let result = encrypted_headers_decrypt(&headers, &dummy_key).unwrap();
    assert!(result.is_empty());
}

#[test]
fn full_encrypted_request_response_cycle() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let built = recipient
        .request_builder()
        .header("x-request-id", "cycle-1")
        .encrypted_header("x-secret", "classified")
        .encrypted_payload(b"full cycle request data".to_vec())
        .build()
        .unwrap();

    let eph_pub: [u8; 32] = built.payload[..32].try_into().unwrap();
    let shared_key = keypair.derive_shared_key(&eph_pub);

    let decrypted_request = nats_micro::encryption::ServiceKeyPair::decrypt_with_shared_key(
        &shared_key,
        &built.payload,
    )
    .unwrap();
    assert_eq!(decrypted_request, b"full cycle request data");

    let decrypted_headers = encrypted_headers_decrypt(&built.headers, &shared_key).unwrap();
    assert_eq!(decrypted_headers.get("x-secret").unwrap(), "classified");

    let response_payload = b"full cycle response";
    let encrypted_response = keypair
        .encrypt_response(response_payload, &eph_pub)
        .unwrap();

    let decrypted_response = built.context.decrypt_response(&encrypted_response).unwrap();
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

#[test]
fn request_builder_encrypted_payload_round_trip() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let built = recipient
        .request_builder()
        .encrypted_payload(b"encrypted via builder".to_vec())
        .build()
        .unwrap();

    assert!(built.payload.len() > b"encrypted via builder".len());
    let decrypted = keypair.decrypt(&built.payload).unwrap();
    assert_eq!(decrypted, b"encrypted via builder");
}

#[test]
fn request_builder_plain_payload_is_unencrypted() {
    let recipient = {
        let keypair = ServiceKeyPair::generate();
        ServiceRecipient::from_bytes(keypair.public_key_bytes())
    };

    let built = recipient
        .request_builder()
        .payload(b"hello plain".to_vec())
        .build()
        .unwrap();

    assert_eq!(&*built.payload, b"hello plain");
}

#[test]
fn request_builder_no_payload_produces_empty_bytes() {
    let recipient = {
        let keypair = ServiceKeyPair::generate();
        ServiceRecipient::from_bytes(keypair.public_key_bytes())
    };

    let built = recipient.request_builder().build().unwrap();
    assert!(built.payload.is_empty());
}

#[test]
fn signature_present_on_all_built_requests() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let plain = recipient
        .request_builder()
        .payload(b"data".to_vec())
        .build()
        .unwrap();
    assert!(plain.headers.get("x-signature").is_some());

    let encrypted = recipient
        .request_builder()
        .encrypted_payload(b"data".to_vec())
        .encrypted_header("k", "v")
        .build()
        .unwrap();
    assert!(encrypted.headers.get("x-signature").is_some());

    let empty = recipient.request_builder().build().unwrap();
    assert!(empty.headers.get("x-signature").is_some());
}

#[test]
fn signature_round_trip() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let ctx = recipient.begin();

    let sig = compute_signature(ctx.shared_secret(), b"test payload", Some("headers-blob"));
    verify_signature(
        ctx.shared_secret(),
        b"test payload",
        Some("headers-blob"),
        &sig,
    )
    .expect("valid signature");
}

#[test]
fn signature_fails_with_wrong_key() {
    let kp1 = ServiceKeyPair::generate();
    let kp2 = ServiceKeyPair::generate();
    let r1 = ServiceRecipient::from_bytes(kp1.public_key_bytes());
    let r2 = ServiceRecipient::from_bytes(kp2.public_key_bytes());
    let c1 = r1.begin();
    let c2 = r2.begin();

    let sig = compute_signature(c1.shared_secret(), b"data", None);
    assert!(verify_signature(c2.shared_secret(), b"data", None, &sig).is_err());
}

#[test]
fn signature_fails_with_tampered_payload() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let ctx = recipient.begin();

    let sig = compute_signature(ctx.shared_secret(), b"original", None);
    assert!(verify_signature(ctx.shared_secret(), b"tampered", None, &sig).is_err());
}

#[test]
fn signature_fails_with_tampered_headers() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let ctx = recipient.begin();

    let sig = compute_signature(ctx.shared_secret(), b"data", Some("original"));
    assert!(verify_signature(ctx.shared_secret(), b"data", Some("tampered"), &sig).is_err());
}

#[test]
fn signature_with_none_headers_differs_from_some() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let ctx = recipient.begin();

    let sig_none = compute_signature(ctx.shared_secret(), b"data", None);
    let sig_some = compute_signature(ctx.shared_secret(), b"data", Some("headers"));
    assert_ne!(sig_none, sig_some);
}

#[test]
fn request_builder_includes_signature() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .payload(b"test".to_vec())
        .build()
        .unwrap();

    assert!(built.headers.get("x-signature").is_some());
    assert!(built.headers.get("x-ephemeral-pub-key").is_some());
}

#[test]
fn request_builder_signature_verifies() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .encrypted_header("authorization", "Bearer tok")
        .encrypted_payload(b"signed data".to_vec())
        .build()
        .unwrap();

    let shared_key = keypair.derive_shared_key(&built.context.ephemeral_pub_bytes());
    let sig_value = built.headers.get("x-signature").unwrap();
    let signature = STANDARD.decode(sig_value.as_str().as_bytes()).unwrap();
    let enc_hdr_val = built
        .headers
        .get("x-encrypted-headers")
        .map(|value| value.as_str());
    verify_signature(&shared_key, &built.payload, enc_hdr_val, &signature)
        .expect("signature should verify");
}

#[test]
fn request_builder_no_client_errors() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let builder = recipient.request_builder().payload(b"data".to_vec());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = rt.block_on(builder.publish("test.subject")).unwrap_err();
    assert!(matches!(err, EncryptionError::NoClient));
}
