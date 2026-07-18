use nats_micro::{message, service, testing::TestApp};

#[message]
pub struct Ping<'a> {
    pub value: &'a str,
}

#[message]
pub struct Pong {
    pub value: String,
}

#[service(name = "in-memory-example", version = "1.0.0")]
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
    let app = TestApp::stateless().serve(PingService).start();
    let pong = app
        .client::<PingService>()
        .ping(&Ping { value: "hello" })
        .await?;
    assert_eq!(pong.value, "hello");
    Ok(())
}
