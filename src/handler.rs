use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    auth::BoxAuthUser,
    error::{IntoNatsError, NatsErrorResponse},
    extractors::FromRequest,
    response::{IntoNatsResponse, NatsResponse},
    state::StateMap,
    request::NatsRequest,
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

// Handler impl: 0 args
impl<F, Fut, Res, Err> Handler<()> for F
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    Res: IntoNatsResponse + Send,
    Err: IntoNatsError + Send,
{
    fn call(&self, ctx: RequestContext) -> HandlerFuture {
        let f = self.clone();
        let request_id = ctx.request.request_id.clone();
        Box::pin(async move {
            let res = f()
                .await
                .map_err(|e| e.into_nats_error(request_id.clone()))?;
            res.into_response(request_id)
        })
    }
}

// Handler impl: 1 arg
impl<F, Fut, Res, Err, A1> Handler<(A1,)> for F
where
    F: Fn(A1) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    Res: IntoNatsResponse + Send,
    Err: IntoNatsError + Send,
    A1: FromRequest,
{
    fn call(&self, ctx: RequestContext) -> HandlerFuture {
        let request_id = ctx.request.request_id.clone();
        let a1 = match A1::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let f = self.clone();
        Box::pin(async move {
            let res = f(a1)
                .await
                .map_err(|e| e.into_nats_error(request_id.clone()))?;
            res.into_response(request_id)
        })
    }
}

// Handler impl: 2 args
impl<F, Fut, Res, Err, A1, A2> Handler<(A1, A2)> for F
where
    F: Fn(A1, A2) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    Res: IntoNatsResponse + Send,
    Err: IntoNatsError + Send,
    A1: FromRequest,
    A2: FromRequest,
{
    fn call(&self, ctx: RequestContext) -> HandlerFuture {
        let request_id = ctx.request.request_id.clone();
        let a1 = match A1::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a2 = match A2::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let f = self.clone();
        Box::pin(async move {
            let res = f(a1, a2)
                .await
                .map_err(|e| e.into_nats_error(request_id.clone()))?;
            res.into_response(request_id)
        })
    }
}

// Handler impl: 3 args
impl<F, Fut, Res, Err, A1, A2, A3> Handler<(A1, A2, A3)> for F
where
    F: Fn(A1, A2, A3) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    Res: IntoNatsResponse + Send,
    Err: IntoNatsError + Send,
    A1: FromRequest,
    A2: FromRequest,
    A3: FromRequest,
{
    fn call(&self, ctx: RequestContext) -> HandlerFuture {
        let request_id = ctx.request.request_id.clone();
        let a1 = match A1::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a2 = match A2::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a3 = match A3::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let f = self.clone();
        Box::pin(async move {
            let res = f(a1, a2, a3)
                .await
                .map_err(|e| e.into_nats_error(request_id.clone()))?;
            res.into_response(request_id)
        })
    }
}

// Handler impl: 4 args
impl<F, Fut, Res, Err, A1, A2, A3, A4> Handler<(A1, A2, A3, A4)> for F
where
    F: Fn(A1, A2, A3, A4) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    Res: IntoNatsResponse + Send,
    Err: IntoNatsError + Send,
    A1: FromRequest,
    A2: FromRequest,
    A3: FromRequest,
    A4: FromRequest,
{
    fn call(&self, ctx: RequestContext) -> HandlerFuture {
        let request_id = ctx.request.request_id.clone();
        let a1 = match A1::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a2 = match A2::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a3 = match A3::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let a4 = match A4::from_request(&ctx) {
            Ok(v) => v,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let f = self.clone();
        Box::pin(async move {
            let res = f(a1, a2, a3, a4)
                .await
                .map_err(|e| e.into_nats_error(request_id.clone()))?;
            res.into_response(request_id)
        })
    }
}
