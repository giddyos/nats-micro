use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("outer failed")]
    Outer {
        source: std::io::Error,
        #[source]
        cause: std::io::Error,
    },
}

fn main() {}
