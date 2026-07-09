use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("outer failed")]
    Outer {
        #[source]
        source: std::io::Error,
    },
}

fn assert_error<E: std::error::Error>() {}

fn main() {
    assert_error::<DemoError>();
    let err = DemoError::Outer {
        source: std::io::Error::other("boom"),
    };
    assert!(std::error::Error::source(&err).is_some());
}
