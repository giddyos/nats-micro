use nats_micro::{message, service, testing::LiveTestApp};

#[message]
pub struct Ping<'a> {
    pub value: &'a str,
}

#[message]
pub struct Pong {
    pub value: String,
}

#[service(name = "live-example", version = "1.0.0")]
impl PingService {
    #[request("ping")]
    async fn ping(input: Ping<'_>) -> Pong {
        Pong {
            value: input.value.to_owned(),
        }
    }
}

#[nats_micro::main]
async fn main() -> nats_micro::Result<()> {
    let live = LiveTestApp::new().serve(PingService).start().await?;
    let pong = live
        .client::<PingService>()
        .ping(&Ping { value: "hello" })
        .await?;
    assert_eq!(pong.value, "hello");
    live.shutdown().await
}
