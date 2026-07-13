use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("io failed")]
    Io(#[from(foo)] std::io::Error),
}

fn main() {}
