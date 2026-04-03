use std::{future::Future, ops::Deref, sync::Arc};

use nats_micro_shared::FrameworkError;
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
                NatsErrorResponse::framework(FrameworkError::Unauthorized, "missing credentials")
                    .with_request_id(request_id)
            }
            AuthError::InvalidCredentials => {
                NatsErrorResponse::framework(FrameworkError::Unauthorized, "invalid credentials")
                    .with_request_id(request_id)
            }
            AuthError::Forbidden => {
                NatsErrorResponse::framework(FrameworkError::Forbidden, "forbidden")
                    .with_request_id(request_id)
            }
            AuthError::Other(msg) => {
                NatsErrorResponse::framework(FrameworkError::Unauthorized, msg)
                    .with_request_id(request_id)
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
