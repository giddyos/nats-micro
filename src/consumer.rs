use crate::handler::HandlerFn;

#[derive(Clone)]
pub struct ConsumerDefinition {
    pub stream: String,
    pub durable: String,
    pub filter_subject: String,
    pub ack_on_success: bool,
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
