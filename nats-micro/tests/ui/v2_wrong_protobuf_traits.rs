use nats_micro::{Proto, service};

struct NotProtobuf;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn decode(input: Proto<NotProtobuf>) -> Proto<NotProtobuf> {
        input
    }
}

fn main() {}
