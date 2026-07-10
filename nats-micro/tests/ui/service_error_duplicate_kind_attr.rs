use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[kind("ONE")]
    #[kind("TWO")]
    #[error("bad")]
    Bad,
}

fn main() {}
