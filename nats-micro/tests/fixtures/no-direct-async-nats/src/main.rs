fn app(client: nats_micro::NatsClient) -> nats_micro::NatsApp {
    nats_micro::NatsApp::new(client)
}

fn headers() -> nats_micro::NatsHeaderMap {
    nats_micro::NatsHeaderMap::new()
}

fn consumer_config() -> nats_micro::NatsConsumerConfig {
    nats_micro::NatsConsumerConfig::default()
}

fn main() {
    let _ = app;
    let _ = headers();
    let _ = consumer_config();
}
