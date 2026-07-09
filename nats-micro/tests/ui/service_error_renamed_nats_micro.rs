extern crate nats_micro as nm;

use nm::service_error;

#[service_error]
pub enum DemoError {
    #[error("empty")]
    Empty,
}

fn assert_error<E: std::error::Error>() {}

fn main() {
    assert_error::<DemoError>();
}
