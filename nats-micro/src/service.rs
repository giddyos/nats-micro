use std::future::Future;

/// A generated compile-time service definition.
pub trait StaticService<S>: Send + Sync + 'static {
    const SPEC: crate::ServiceSpec;
}

/// Generated local routing over the same concrete endpoint dispatch functions.
pub trait LocalService<S>: StaticService<S> {
    fn dispatch_local<'a>(
        state: &'a S,
        operation: usize,
        request: crate::Request<'a>,
    ) -> impl Future<Output = crate::DispatchResult> + Send + 'a;
}

/// Static metadata for a generated operation marker.
#[derive(Debug, Clone, Copy)]
pub struct OperationMarker {
    pub index: usize,
    pub spec: &'static crate::OperationSpec,
}

/// A generated publish-only operation.
pub trait PublishOperation: Send + Sync + 'static {
    const SPEC: crate::OperationSpec;
}
