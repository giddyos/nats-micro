use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[code(404)]
    #[code(409)]
    #[error("bad")]
    Bad,
}

fn main() {}
