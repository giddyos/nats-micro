use nats_micro::service_error;
use thiserror::Error;

#[service_error]
#[derive(Debug, Error)]
pub enum DemoError {
    #[error("empty")]
    Empty,
}

fn main() {}
