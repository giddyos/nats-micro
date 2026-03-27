pub mod app;
pub mod auth;
pub mod consumer;
pub mod error;
pub mod extractors;
pub mod handler;
pub mod prelude;
pub mod registry;
pub mod request;
pub mod response;
pub mod service;
pub mod state;
pub mod utils;

pub use app::NatsApp;
pub use auth::{Auth, AuthConfig, AuthError};
pub use consumer::{ConsumerDefinition, ConsumerHandlerFn};
pub use error::{IntoNatsError, NatsError, NatsErrorResponse};
pub use extractors::{FromRequest, FromSubjectParam, Json, Proto, RequestId, State, SubjectParam};
pub use handler::{HandlerFn, RequestContext};
pub use inventory;
pub use request::NatsRequest;
pub use response::{IntoNatsResponse, NatsResponse};
pub use service::{
    ConsumerInfo, EndpointDefinition, EndpointInfo, NatsService, ParamInfo, ServiceDefinition,
    ServiceMetadata,
};
pub use state::StateMap;

pub use nats_micro_macros::{consumer, endpoint, service, service_error, service_handlers};

pub mod __private {
    pub use crate::error::IntoNatsError;
    pub use crate::extractors::FromRequest;
    pub use crate::handler::into_handler_fn;
    pub use crate::handler::{HandlerFuture, RequestContext};
    pub use crate::inventory;
    pub use crate::registry::ServiceRegistration;
    pub use crate::response::{IntoNatsResponse, NatsResponse};
    pub use crate::service::{
        ConsumerInfo, EndpointInfo, NatsService, ParamInfo, ServiceDefinition,
    };
}
