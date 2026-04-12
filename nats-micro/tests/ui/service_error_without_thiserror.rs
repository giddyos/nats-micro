use nats_micro::{FromNatsErrorResponse, IntoNatsError, ServiceErrorMatch, service_error};

#[service_error]
enum DemoError {
    #[error("numbers must not be empty")]
    EmptyNumbers,

    #[error("retry after {0} seconds")]
    RetryAfter(u64),

    #[error("invalid range {min}..{max}")]
    InvalidRange { min: i64, max: i64 },
}

fn main() {
    let response = DemoError::InvalidRange { min: 1, max: 4 }.into_nats_error("req-1".to_string());
    let round_trip = DemoError::from_nats_error_response(response);

    assert!(matches!(
        round_trip,
        ServiceErrorMatch::Typed(DemoError::InvalidRange { min: 1, max: 4 })
    ));

    let _ = format!("{}", DemoError::RetryAfter(3));
    let _ = format!("{:?}", DemoError::EmptyNumbers);
}