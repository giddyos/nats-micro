use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("a")]
    FooBar,

    #[error("b")]
    FOO_BAR,
}

fn main() {}
