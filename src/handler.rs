use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    auth::BoxAuthUser,
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

#[derive(Clone)]
pub struct RequestContext {
    pub request: NatsRequest,
    pub states: StateMap,
    pub user: Option<BoxAuthUser>,
    pub subject_template: Option<String>,
    pub current_param_name: Option<String>,
}

impl RequestContext {
    pub fn with_param_name(&self, name: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.current_param_name = Some(name.into());
        next
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
        #[allow(non_snake_case)] // T1, T2... used as variable bindings
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
                $(
                    let $ty = match $ty::from_request(&ctx) {
                        Ok(v) => v,
                        Err(e) => return Box::pin(async move { Err(e) }),
                    };
                )*
                let f = self.clone();
                Box::pin(async move {
                    let res = f($($ty),*)
                        .await
                        .map_err(|e| e.into_nats_error(request_id.clone()))?;
                    res.into_response(request_id)
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

