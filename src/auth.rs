use std::{future::Future, ops::Deref, sync::Arc};

use thiserror::Error;

use crate::{error::NatsErrorResponse, handler::RequestContext};

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
                NatsErrorResponse::forbidden("FORBIDDEN", "forbidden").with_request_id(request_id)
            }
            AuthError::Other(msg) => {
                NatsErrorResponse::unauthorized("UNAUTHORIZED", msg).with_request_id(request_id)
            }
        }
    }
}

pub trait FromAuthRequest: Sized + Send + Sync + 'static {
    fn from_auth_request(
        ctx: &RequestContext,
    ) -> impl Future<Output = Result<Self, AuthError>> + Send;
}

#[derive(Clone)]
pub struct Auth<U>(Arc<U>);

impl<U> Auth<U> {
    pub fn new(inner: U) -> Self {
        Self(Arc::new(inner))
    }

    pub fn from_arc(inner: Arc<U>) -> Self {
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
