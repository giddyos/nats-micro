use async_nats::jetstream::consumer::push;

use crate::handler::HandlerFn;
use crate::service::AuthPolicy;

#[derive(Clone)]
pub struct ConsumerDefinition {
    pub stream: String,
    pub durable: String,
    pub auth_policy: AuthPolicy,
    pub concurrency_limit: Option<u64>,
    pub config: push::Config,
    pub handler: HandlerFn,
}

impl ConsumerDefinition {
    #[must_use]
    pub fn auth_required(&self) -> bool {
        self.auth_policy.auth_required()
    }
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
