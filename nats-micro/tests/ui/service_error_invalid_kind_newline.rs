use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[kind("BAD\nKIND")]
    #[error("bad")]
    Bad,
}

fn main() {}
