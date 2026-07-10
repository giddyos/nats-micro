use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[internal(false)]
    #[error("bad")]
    Bad,
}

fn main() {}
