use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("bad {password}")]
    Bad {
        username: String,
        #[details(secret)]
        password: String,
    },
}

fn main() {}
