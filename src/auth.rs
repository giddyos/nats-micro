use std::{ops::Deref, sync::Arc};

use futures::future::BoxFuture;
use thiserror::Error;

use crate::{error::NatsErrorResponse, request::NatsRequest};

pub type BoxAuthUser = Arc<dyn std::any::Any + Send + Sync>;

type AuthResolverFn =
    dyn Fn(&NatsRequest) -> BoxFuture<'static, Result<BoxAuthUser, AuthError>> + Send + Sync;

#[derive(Clone)]
pub struct AuthConfig {
    resolver: Arc<AuthResolverFn>,
}

impl AuthConfig {
    pub fn new<U, F, Fut>(resolver: F) -> Self
    where
        U: Send + Sync + 'static,
        F: Fn(&NatsRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<U, AuthError>> + Send + 'static,
    {
        Self {
            resolver: Arc::new(move |req| {
                let fut = resolver(req);
                Box::pin(async move {
                    let user = fut.await?;
                    Ok(Arc::new(user) as BoxAuthUser)
                })
            }),
        }
    }

    pub async fn resolve(&self, req: &NatsRequest) -> Result<BoxAuthUser, AuthError> {
        (self.resolver)(req).await
    }
}

#[derive(Debug, Error, Clone)]
pub enum AuthError {
    #[error("missing credentials")]
    MissingCredentials,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("forbidden")]
    Forbidden,
    #[error("auth error: {0}")]
    Other(String),
}

impl crate::error::IntoNatsError for AuthError {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        match self {
            AuthError::MissingCredentials => {
                NatsErrorResponse::unauthorized("UNAUTHORIZED", "missing credentials")
                    .with_request_id(request_id)
            }
            AuthError::InvalidCredentials => {
                NatsErrorResponse::unauthorized("UNAUTHORIZED", "invalid credentials")
                    .with_request_id(request_id)
            }
            AuthError::Forbidden => {
                NatsErrorResponse::forbidden("FORBIDDEN", "forbidden")
                    .with_request_id(request_id)
            }
            AuthError::Other(msg) => {
                NatsErrorResponse::unauthorized("UNAUTHORIZED", msg).with_request_id(request_id)
            }
        }
    }
}

#[derive(Clone)]
pub struct Auth<U>(Arc<U>);

impl<U> Auth<U> {
    pub fn new(inner: Arc<U>) -> Self {
        Self(inner)
    }

    pub fn into_inner(self) -> Arc<U> {
        self.0
    }
}

impl<U> Deref for Auth<U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
