use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[kind("BAD KIND")]
    #[error("bad")]
    Bad,
}

fn main() {}
