use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error(transparent)]
    Inner(std::io::Error, String),
}

fn main() {}
