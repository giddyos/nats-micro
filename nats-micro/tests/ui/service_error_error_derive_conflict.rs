use nats_micro::service_error;

#[service_error]
#[derive(nats_micro::Error)]
pub enum DemoError {
    #[error("empty")]
    Empty,
}

fn main() {}
