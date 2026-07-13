use nats_micro::service_error;

#[service_error]
pub enum DemoError {
    #[error("db failed")]
    Database(#[from] std::io::Error),

    #[code(500)]
    #[kind("INTERNAL_ERROR")]
    #[error("public internal-ish error")]
    PublicInternal,
}

fn main() {}
