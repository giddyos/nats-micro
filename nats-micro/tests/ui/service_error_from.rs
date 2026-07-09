use nats_micro::{service_error, IntoNatsError};

#[service_error]
pub enum DemoError {
    #[error("io failed")]
    Io(#[from] std::io::Error),
}

fn assert_error<E: std::error::Error>() {}

fn main() {
    assert_error::<DemoError>();
    let err = DemoError::from(std::io::Error::other("boom"));
    let response = err.into_nats_error("req-1".to_string());
    assert_eq!(response.code, 500);
    assert_eq!(response.message, "an internal error occurred");
}
