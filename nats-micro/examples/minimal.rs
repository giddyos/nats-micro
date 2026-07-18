use nats_micro::prelude::*;

#[message]
pub struct Ping<'a> {
    pub value: &'a str,
}

#[message]
pub struct Pong {
    pub value: String,
}

#[service(name = "ping", version = "1.0.0")]
impl PingService {
    #[request]
    async fn ping(input: Ping<'_>) -> Pong {
        Pong {
            value: input.value.to_owned(),
        }
    }
}

#[nats_micro::main]
async fn main() -> nats_micro::anyhow::Result<()> {
    App::stateless().serve(PingService).run().await
}
