use async_nats::jetstream::consumer::push;

use crate::handler::HandlerFn;

#[derive(Clone)]
pub struct ConsumerDefinition {
    pub stream: String,
    pub durable: String,
    pub auth_required: bool,
    pub config: push::Config,
    pub handler: HandlerFn,
}

#[derive(Clone)]
pub struct ConsumerHandlerFn {
    pub inner: HandlerFn,
}

impl ConsumerHandlerFn {
    pub fn new(inner: HandlerFn) -> Self {
        Self { inner }
    }
}
