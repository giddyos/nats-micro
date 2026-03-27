mod app;
mod auth;
mod consumer;
mod error;
mod extractors;
mod handler;
mod prelude;
mod registry;
mod request;
mod response;
mod service;
mod state;
mod utils;

pub use app::NatsApp;
pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
pub use auth::{Auth, AuthConfig, AuthError};
pub use consumer::{ConsumerDefinition, ConsumerHandlerFn};
pub use error::{IntoNatsError, NatsError, NatsErrorResponse};
pub use extractors::{
    FromPayload, FromRequest, FromSubjectParam, Json, Payload, Proto, RequestId, State,
    SubjectParam,
};
pub use handler::{HandlerFn, RequestContext};
pub use request::NatsRequest;
pub use response::{IntoNatsResponse, NatsResponse};
pub use service::{
    ConsumerInfo, EndpointDefinition, EndpointInfo, NatsService, ParamInfo, ServiceDefinition,
    ServiceMetadata,
};
pub use state::StateMap;

pub use bytes::Bytes;
pub use nats_micro_macros::{service, service_error, service_handlers};

#[doc(hidden)]
pub mod __macros {
    pub use crate::error::IntoNatsError;
    pub use crate::extractors::FromRequest;
    pub use crate::handler::into_handler_fn;
    pub use crate::handler::{HandlerFuture, RequestContext};
    pub use crate::registry::ServiceRegistration;
    pub use crate::response::{IntoNatsResponse, NatsResponse};
    pub use crate::service::{
        ConsumerInfo, EndpointInfo, NatsService, ParamInfo, ServiceDefinition,
    };
    pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
    pub use inventory;
}
