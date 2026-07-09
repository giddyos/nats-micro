use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("outer failed")]
    Outer {
        #[source]
        source: String,
    },
}

fn main() {}
