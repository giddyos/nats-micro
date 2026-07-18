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
