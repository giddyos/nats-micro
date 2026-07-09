use nm::service_error;

#[service_error]
pub enum DemoError {
    #[error("empty")]
    Empty,

    #[error("io failed")]
    Io(#[from] std::io::Error),
}

pub fn assert_error<E: std::error::Error>() {}

fn main() {
    assert_error::<DemoError>();
    let _ = DemoError::from(std::io::Error::other("boom"));
}
