use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("io failed for {1}")]
    Io(#[from] std::io::Error, String),
}

fn main() {}
