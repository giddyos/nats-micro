use nats_micro::prelude::*;

#[message]
pub struct Status {
    pub ready: bool,
}

#[service(name = "application-example", version = "1.0.0")]
impl StatusService {
    #[request("status")]
    async fn status() -> Status {
        Status { ready: true }
    }
}

nats_micro::application! {
    state: (),
    profile: Production,
    services: [StatusService],
}
