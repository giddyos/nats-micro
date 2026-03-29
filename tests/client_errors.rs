use nats_micro::{
    ClientError, ClientTransportError, FromNatsErrorResponse, IntoNatsError, NatsErrorResponse,
    ServiceErrorMatch, ServiceKeyPair, ServiceRecipient, service_error,
};
use prost::Message;
use thiserror::Error;

#[service_error]
#[derive(Debug, Error)]
enum ClientTestError {
    #[error("numbers must not be empty")]
    #[code(400)]
    EmptyNumbers,

    #[error("retry after {0} seconds")]
    #[code(429)]
    RetryAfter(u64),

    #[error("invalid range {min}..{max}")]
    #[code(422)]
    InvalidRange { min: i64, max: i64 },
}

#[derive(Clone, PartialEq, Message)]
struct ProtoResponse {
    #[prost(int64, tag = "1")]
    total: i64,
}

#[test]
fn service_error_round_trips_from_nats_error_response() {
    let response = ClientTestError::EmptyNumbers.into_nats_error("req-typed".to_string());
    let error = ClientTestError::from_nats_error_response(response);

    assert!(matches!(
        error,
        ServiceErrorMatch::Typed(ClientTestError::EmptyNumbers)
    ));
}

#[test]
fn proto_deserializer_maps_service_error_payloads() {
    let payload =
        serde_json::to_vec(&ClientTestError::EmptyNumbers.into_nats_error("req-proto".to_string()))
            .expect("serialize error response");

    let result = nats_micro::__macros::deserialize_proto_response::<ProtoResponse, ClientTestError>(
        &payload,
    );

    assert!(matches!(
        result,
        Err(ClientError::Service(ClientTestError::EmptyNumbers))
    ));
}

#[test]
fn structured_tuple_service_errors_round_trip() {
    let response = ClientTestError::RetryAfter(30).into_nats_error("req-retry".to_string());

    assert_eq!(response.details, Some(serde_json::json!([30])));
    assert!(matches!(
        ClientTestError::from_nats_error_response(response),
        ServiceErrorMatch::Typed(ClientTestError::RetryAfter(30))
    ));
}

#[test]
fn structured_named_service_errors_round_trip() {
    let response =
        ClientTestError::InvalidRange { min: 4, max: 9 }.into_nats_error("req-range".to_string());

    assert_eq!(response.details, Some(serde_json::json!([4, 9])));
    assert!(matches!(
        ClientTestError::from_nats_error_response(response),
        ServiceErrorMatch::Typed(ClientTestError::InvalidRange { min: 4, max: 9 })
    ));
}

#[test]
fn structured_service_errors_without_details_stay_untyped() {
    let response = NatsErrorResponse::new(422, "InvalidRange", "invalid range 4..9", "req-missing");

    assert!(matches!(
        ClientTestError::from_nats_error_response(response),
        ServiceErrorMatch::Untyped(_)
    ));
}

#[cfg(feature = "encryption")]
#[test]
fn encrypted_response_falls_back_to_plain_service_error_payloads() {
    let keypair = ServiceKeyPair::generate();
    let recipient = ServiceRecipient::from_bytes(keypair.public_key_bytes());
    let built = recipient
        .request_builder()
        .encrypted_payload(br#"{\"numbers\":[1]}"#.to_vec())
        .build()
        .expect("build encrypted request");

    let payload = serde_json::to_vec(
        &ClientTestError::EmptyNumbers.into_nats_error("req-encrypted".to_string()),
    )
    .expect("serialize error response");

    let result =
        nats_micro::__macros::decrypt_client_response::<ClientTestError>(&built.context, &payload);

    assert!(matches!(
        result,
        Err(ClientError::Service(ClientTestError::EmptyNumbers))
    ));
}

#[test]
fn invalid_proto_payload_is_reported_as_transport_error() {
    let result = nats_micro::__macros::deserialize_proto_response::<ProtoResponse, NatsErrorResponse>(
        b"not-protobuf-and-not-json-error",
    );

    assert!(matches!(
        result,
        Err(ClientError::Transport(ClientTransportError::Deserialize(_)))
    ));
}
