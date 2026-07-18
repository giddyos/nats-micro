use std::future::Future;

use crate::{ErrorReply, OperationSpec, Request, Response};

pub type DispatchResult = Result<Response, ErrorReply>;

/// A concrete request endpoint used only through monomorphized dispatch.
pub trait RequestEndpoint<S>: Send + Sync + 'static {
    const SPEC: OperationSpec;

    fn call<'a>(
        state: &'a S,
        request: Request<'a>,
    ) -> impl Future<Output = DispatchResult> + Send + 'a;
}

/// A concrete request endpoint whose generated dispatcher owns decrypted
/// request material for the duration of the handler future.
#[cfg(feature = "encryption")]
pub trait EncryptedRequestEndpoint<S>: Send + Sync + 'static {
    const SPEC: OperationSpec;

    fn call<'a>(
        state: &'a S,
        keypair: &'a crate::ServiceKeyPair,
        request: Request<'a>,
    ) -> impl Future<Output = DispatchResult> + Send + 'a;
}

/// A concrete core-NATS subscription handler.
pub trait SubscriptionHandler<S>: Send + Sync + 'static {
    const SPEC: OperationSpec;

    fn call<'a>(
        state: &'a S,
        request: Request<'a>,
    ) -> impl Future<Output = Result<(), ErrorReply>> + Send + 'a;
}
