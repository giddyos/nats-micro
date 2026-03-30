use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    error::{IntoNatsError, NatsErrorResponse},
    extractors::FromRequest,
    request::NatsRequest,
    response::{IntoNatsResponse, NatsResponse},
    state::StateMap,
};

pub type HandlerFuture =
    Pin<Box<dyn Future<Output = Result<NatsResponse, NatsErrorResponse>> + Send + 'static>>;

#[derive(Clone)]
pub struct HandlerFn {
    inner: Arc<dyn Fn(RequestContext) -> HandlerFuture + Send + Sync>,
}

impl HandlerFn {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(RequestContext) -> HandlerFuture + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    pub fn call(&self, ctx: RequestContext) -> HandlerFuture {
        (self.inner)(ctx)
    }
}

pub struct RequestContext {
    pub request: NatsRequest,
    pub states: StateMap,
    pub subject_template: Option<String>,
    pub current_param_name: Option<String>,
    #[cfg(feature = "encryption")]
    pub(crate) ephemeral_pub: Option<[u8; 32]>,
}

impl Clone for RequestContext {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            states: self.states.clone(),
            subject_template: self.subject_template.clone(),
            current_param_name: self.current_param_name.clone(),
            #[cfg(feature = "encryption")]
            ephemeral_pub: self.ephemeral_pub,
        }
    }
}

impl RequestContext {
    pub fn new(request: NatsRequest, states: StateMap, subject_template: Option<String>) -> Self {
        Self {
            request,
            states,
            subject_template,
            current_param_name: None,
            #[cfg(feature = "encryption")]
            ephemeral_pub: None,
        }
    }

    pub fn with_param_name(&self, name: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.current_param_name = Some(name.into());
        next
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub fn __with_ephemeral_pub(mut self, ephemeral_pub: Option<[u8; 32]>) -> Self {
        self.ephemeral_pub = ephemeral_pub;
        self
    }
}

pub trait Handler<Args>: Send + Sync + 'static {
    fn call(&self, ctx: RequestContext) -> HandlerFuture;
}

pub fn into_handler_fn<H, Args>(handler: H) -> HandlerFn
where
    H: Handler<Args> + Clone,
{
    HandlerFn::new(move |ctx| Handler::call(&handler, ctx))
}

macro_rules! impl_handler {
    ([$($ty:ident),*]) => {
        #[allow(non_snake_case)]
        impl<F, Fut, Res, Err, $($ty,)*> Handler<($($ty,)*)> for F
        where
            F: Fn($($ty),*) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = Result<Res, Err>> + Send + 'static,
            Res: IntoNatsResponse + Send,
            Err: IntoNatsError + Send,
            $($ty: FromRequest,)*
        {
            fn call(&self, ctx: RequestContext) -> HandlerFuture {
                let request_id = ctx.request.request_id.clone();
                let f = self.clone();
                Box::pin(async move {
                    $(
                        let $ty = match $ty::from_request(&ctx).await {
                            Ok(v) => v,
                            Err(e) => return Err(e),
                        };
                    )*
                    let res = f($($ty),*)
                        .await
                        .map_err(|e| e.into_nats_error(request_id.clone()))?;
                    res.into_response(&ctx)
                })
            }
        }
    };
}

macro_rules! all_the_tuples {
    ($name:ident) => {
        $name!([]);
        $name!([T1]);
        $name!([T1, T2]);
        $name!([T1, T2, T3]);
        $name!([T1, T2, T3, T4]);
        $name!([T1, T2, T3, T4, T5]);
        $name!([T1, T2, T3, T4, T5, T6]);
        $name!([T1, T2, T3, T4, T5, T6, T7]);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8]);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9]);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10]);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11]);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12]);
    };
}

all_the_tuples!(impl_handler);
