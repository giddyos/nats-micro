use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[kind(123)]
    #[error("bad")]
    Bad,
}

fn main() {}
