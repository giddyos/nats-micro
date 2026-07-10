#[derive(Debug, nats_micro::Error)]
pub enum LocalError {
    #[error("bad config: {0}")]
    BadConfig(String),
}

pub fn assert_error<E: std::error::Error>() {}

#[test]
fn raw_thiserror_reexport_works_with_direct_dependency() {
    assert_error::<LocalError>();
    assert_eq!(LocalError::BadConfig("missing".to_string()).to_string(), "bad config: missing");
}

fn main() {}
