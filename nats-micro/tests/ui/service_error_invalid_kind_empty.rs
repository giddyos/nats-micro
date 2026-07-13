use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[kind("")]
    #[error("bad")]
    Bad,
}

fn main() {}
