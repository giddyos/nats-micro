use std::rc::Rc;

use nats_micro::service;

#[service(name = "demo", version = "1.0.0")]
impl Demo {
    #[request]
    async fn bad() {
        let value = Rc::new("not-send");
        std::future::ready(()).await;
        drop(value);
    }
}

fn main() {}
