use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[code(200)]
    #[error("bad")]
    Bad,
}

fn main() {}
