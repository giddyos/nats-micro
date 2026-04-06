use nats_micro::service;

#[service(name = "invalid-version", version = "1.0")]
struct InvalidVersionService;

fn main() {}
