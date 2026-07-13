use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("outer failed")]
    Outer {
        #[source]
        source: std::io::Error,
        #[backtrace(foo)]
        backtrace: std::backtrace::Backtrace,
    },
}

fn main() {}
