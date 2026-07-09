use nats_micro::service_error;

#[service_error]
enum DemoError {
    #[error("empty")]
    Empty,
}

fn main() {}
