use crate::handler::HandlerFn;
use crate::spec::AuthPolicy;
use std::future::Future;

use crate::{ConsumerSpec, ErrorReply, Request};

/// The acknowledgement decision produced by a static consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumerAction {
    Ack,
    Nack,
    NackAfter(std::time::Duration),
    Term,
}

/// A concrete `JetStream` consumer used only through monomorphized dispatch.
pub trait ConsumerHandler<S>: Send + Sync + 'static {
    const SPEC: ConsumerSpec;

    fn call<'a>(
        state: &'a S,
        request: Request<'a>,
    ) -> impl Future<Output = Result<ConsumerAction, ErrorReply>> + Send + 'a;
}

#[derive(Clone)]
pub struct ConsumerDefinition {
    pub stream: String,
    pub durable: String,
    pub auth_policy: AuthPolicy,
    pub concurrency_limit: Option<u64>,
    pub config: crate::NatsConsumerConfig,
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
