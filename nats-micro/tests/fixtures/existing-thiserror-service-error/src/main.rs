use nats_micro::{FromNatsErrorResponse, IntoNatsError, ServiceErrorMatch, service_error};
use thiserror::Error;

#[service_error]
#[derive(Debug, Error)]
pub enum ExistingUserError {
    #[code(404)]
    #[error("user {id} was not found")]
    NotFound { id: String },

    #[code(409)]
    #[error("email {0} already exists")]
    EmailExists(String),

    #[error("database failed")]
    Database(#[from] std::io::Error),

    #[error(transparent)]
    Parse(#[from] std::num::ParseIntError),
}

#[service_error]
#[derive(Debug, Error)]
pub enum ExistingOrderError {
    #[code(422)]
    #[error("order {order_id} is invalid: {reason}")]
    Invalid { order_id: String, reason: String },
}

fn assert_error<E: std::error::Error>() {}

#[test]
fn existing_thiserror_service_error_wire_behavior() {
    assert_error::<ExistingUserError>();
    assert_error::<ExistingOrderError>();

    let err = ExistingUserError::NotFound {
        id: "user-1".to_string(),
    };
    assert_eq!(err.to_string(), "user user-1 was not found");

    let response = err.into_nats_error("req-1".to_string());
    assert_eq!(response.code, 404);
    assert_eq!(response.kind, "NOT_FOUND");
    assert_eq!(response.message, "user user-1 was not found");

    assert!(matches!(
        ExistingUserError::from_nats_error_response(response),
        ServiceErrorMatch::Typed(ExistingUserError::NotFound { id }) if id == "user-1"
    ));

    let err = ExistingUserError::from(std::io::Error::other("disk"));
    assert!(std::error::Error::source(&err).is_some());
    let response = err.into_nats_error("req-2".to_string());
    assert_eq!(response.code, 500);
    assert_eq!(response.kind, "INTERNAL_ERROR");
    assert_eq!(response.message, "an internal error occurred");

    let parse = "bad".parse::<u64>().expect_err("expected parse error");
    let err = ExistingUserError::from(parse);
    assert!(err.to_string().contains("invalid digit"));
    let response = err.into_nats_error("req-3".to_string());
    assert_eq!(response.code, 500);
    assert_eq!(response.kind, "INTERNAL_ERROR");
    assert_eq!(response.message, "an internal error occurred");
}

fn main() {}
