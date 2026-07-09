use nats_micro::{service_error, IntoNatsError};

#[service_error]
pub enum DemoError {
    #[error(transparent)]
    Inner(std::io::Error),
}

fn assert_error<E: std::error::Error>() {}

fn main() {
    assert_error::<DemoError>();
    let err = DemoError::Inner(std::io::Error::other("boom"));
    assert_eq!(err.to_string(), "boom");
    let response = err.into_nats_error("req-1".to_string());
    assert_eq!(response.code, 500);
}
