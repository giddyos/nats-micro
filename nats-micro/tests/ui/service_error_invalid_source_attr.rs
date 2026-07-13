use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("outer failed")]
    Outer(#[source(foo)] std::io::Error),
}

fn main() {}
