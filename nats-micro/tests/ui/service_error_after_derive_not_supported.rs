use nats_micro::service_error;
use thiserror::Error;

#[derive(Debug, Error)]
#[service_error]
pub enum DemoError {
    #[error("empty")]
    Empty,
}

fn main() {}
