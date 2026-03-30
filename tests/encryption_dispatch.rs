#![cfg(feature = "encryption")]

use async_nats::HeaderMap;
use nats_micro::{
    __test_support, Auth, AuthError, BuiltRequest, Encrypted, FromAuthRequest, FromRequest,
    Headers, IntoNatsResponse, NatsRequest, RequestContext, ServiceKeyPair, ServiceRecipient,
    StateMap,
};

struct DemoUser {
    id: String,
}

impl FromAuthRequest for DemoUser {
    async fn from_auth_request(ctx: &RequestContext) -> Result<Self, AuthError> {
        match ctx.request.headers.get("authorization").map(|value| value.as_str()) {
            Some("Bearer demo-token") => Ok(Self {
                id: "demo-user".to_string(),
            }),
            Some(_) => Err(AuthError::InvalidCredentials),
            None => Err(AuthError::MissingCredentials),
        }
    }
}

fn state_with_keypair(keypair: ServiceKeyPair) -> StateMap {
    StateMap::new().insert(keypair)
}

fn request_with_headers(headers: HeaderMap, payload: &[u8]) -> NatsRequest {
    NatsRequest {
        subject: "secure.demo".to_string(),
        payload: payload.to_vec().into(),
        headers: headers.into(),
        reply: Some("reply.subject".to_string()),
        request_id: "req-header-only".to_string(),
    }
}

#[test]
fn prepare_request_for_dispatch_decrypts_header_only_requests() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .encrypted_header("x-user-id", "user-42")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let eph_pub_bytes = built.context.ephemeral_pub_bytes();
    let (prepared, ephemeral_pub) = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(built.headers, &built.payload),
    )
    .expect("header-only request decrypts");

    assert_eq!(prepared.payload.as_ref(), b"plain payload");
    assert_eq!(ephemeral_pub, Some(eph_pub_bytes));
    assert_eq!(
        prepared
            .headers
            .get("authorization")
            .map(|value| value.as_str()),
        Some("Bearer demo-token")
    );
    assert_eq!(
        prepared
            .headers
            .get("x-user-id")
            .map(|value| value.as_str()),
        Some("user-42")
    );
    assert!(prepared.headers.get("x-encrypted-headers").is_none());
    assert!(prepared.headers.get("x-ephemeral-pub-key").is_none());
    assert!(prepared.headers.get("x-signature").is_none());
}

#[tokio::test]
async fn prepare_request_for_dispatch_runs_before_auth_resolution() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let (prepared, _) = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(built.headers, &built.payload),
    )
    .expect("header-only request decrypts");

    let user = Auth::<DemoUser>::from_request(&RequestContext::new(prepared, StateMap::new(), None))
        .await
        .expect("auth uses decrypted headers");

    assert_eq!(user.id, "demo-user");
}

#[test]
fn header_only_encryption_can_drive_encrypted_response() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let BuiltRequest {
        headers,
        payload,
        context: eph,
    } = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let state = state_with_keypair(keypair);
    let (prepared, ephemeral_pub) = __test_support::prepare_request_for_dispatch_with_state(
        &state,
        request_with_headers(headers, &payload),
    )
    .expect("header-only request decrypts");

    let ctx = RequestContext::new(prepared, state, None).__with_ephemeral_pub(ephemeral_pub);

    let response = Encrypted(String::from("encrypted response"))
        .into_response(&ctx)
        .expect("response encrypts from header-only request");

    let decrypted = eph
        .decrypt_response(&response.payload)
        .expect("client decrypts response");
    assert_eq!(decrypted, b"encrypted response");
    assert_eq!(
        ctx.request
            .headers
            .get("authorization")
            .map(|value| value.as_str()),
        Some("Bearer demo-token")
    );
}

#[test]
fn prepare_request_for_dispatch_rejects_headers_for_wrong_service() {
    let correct_keypair = ServiceKeyPair::generate();
    let wrong_keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(correct_keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let err = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(wrong_keypair),
        request_with_headers(built.headers, &built.payload),
    )
    .expect_err("wrong service key should fail");

    assert_eq!(err.code, 400);
    assert_eq!(err.error, "SIGNATURE_INVALID");
    assert_eq!(err.message, "request signature verification failed");
    assert_eq!(err.request_id, "req-header-only");
}

#[test]
fn tampered_payload_fails_signature_verification() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .encrypted_payload(b"secret payload".to_vec())
        .build()
        .expect("build request");

    let mut tampered_payload = built.payload.to_vec();
    if let Some(last) = tampered_payload.last_mut() {
        *last ^= 0xFF;
    }

    let err = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(built.headers, &tampered_payload),
    )
    .expect_err("tampered payload should fail");

    assert_eq!(err.code, 400);
    assert_eq!(err.error, "SIGNATURE_INVALID");
}

#[test]
fn missing_signature_fails_when_eph_pub_key_present() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let mut headers = async_nats::HeaderMap::new();
    for (name, values) in built.headers.iter() {
        let name_ref: &str = name.as_ref();
        if name_ref.eq_ignore_ascii_case("x-signature") {
            continue;
        }
        for value in values {
            headers.append(name_ref, value.as_str());
        }
    }

    let err = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(headers, &built.payload),
    )
    .expect_err("missing signature should fail");

    assert_eq!(err.code, 400);
    assert_eq!(err.error, "SIGNATURE_MISSING");
}

#[test]
fn plain_request_without_encryption_passes_through() {
    let keypair = ServiceKeyPair::generate();
    let headers = {
        let mut h = HeaderMap::new();
        h.insert("x-request-id", "req-plain");
        h.insert("authorization", "Bearer tok");
        h
    };

    let (prepared, ephemeral_pub) = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(headers, b"plain payload"),
    )
    .expect("plain request passes");

    assert_eq!(prepared.payload.as_ref(), b"plain payload");
    assert_eq!(ephemeral_pub, None);
    assert_eq!(
        prepared
            .headers
            .get("authorization")
            .map(|value| value.as_str()),
        Some("Bearer tok")
    );
}

#[tokio::test]
async fn headers_extractor_tracks_encrypted_precedence() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .header("Authorization", "Bearer plaintext")
        .header("x-plain", "plain-value")
        .encrypted_header("authorization", "Bearer encrypted")
        .encrypted_header("x-secure", "secure-value")
        .payload(b"plain payload".to_vec())
        .build()
        .expect("build request");

    let state = state_with_keypair(keypair);
    let (prepared, ephemeral_pub) = __test_support::prepare_request_for_dispatch_with_state(
        &state,
        request_with_headers(built.headers, &built.payload),
    )
    .expect("request decrypts");

    assert_eq!(
        prepared
            .headers
            .get("authorization")
            .map(|value| value.as_str()),
        Some("Bearer encrypted")
    );

    let ctx = RequestContext::new(prepared, state, None).__with_ephemeral_pub(ephemeral_pub);
    let headers = Headers::from_request(&ctx)
        .await
        .expect("headers extractor should work");

    let authorization = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("authorization"))
        .expect("authorization header present");
    assert_eq!(authorization.value, "Bearer encrypted");
    assert!(authorization.was_encrypted);

    let secure = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("x-secure"))
        .expect("x-secure header present");
    assert_eq!(secure.value, "secure-value");
    assert!(secure.was_encrypted);

    let plain = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("x-plain"))
        .expect("x-plain header present");
    assert_eq!(plain.value, "plain-value");
    assert!(!plain.was_encrypted);
}

#[test]
fn encrypted_payload_rejects_mismatched_header_ephemeral_key() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());

    let built = recipient
        .request_builder()
        .header("x-request-id", "req-header-only")
        .encrypted_header("authorization", "Bearer demo-token")
        .build()
        .expect("build request");

    let payload_recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let encrypted_payload = payload_recipient
        .encrypt(b"encrypted payload")
        .expect("payload encrypts")
        .0;

    let err = __test_support::prepare_request_for_dispatch_with_state(
        &state_with_keypair(keypair),
        request_with_headers(built.headers, &encrypted_payload),
    )
    .expect_err("mismatched payload should fail signature");

    assert_eq!(err.code, 400);
    assert_eq!(err.error, "SIGNATURE_INVALID");
}
