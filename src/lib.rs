mod app;
mod auth;
mod consumer;
#[cfg(feature = "encryption")]
mod encrypted;
#[cfg(feature = "encryption")]
mod encrypted_headers;
#[cfg(feature = "encryption")]
pub mod encryption;
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
    FromPayload, FromRequest, FromSubjectParam, Json, Payload, Proto, RequestId, State, Subject,
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

#[cfg(feature = "encryption")]
pub use encrypted::Encrypted;
#[cfg(feature = "encryption")]
pub use encrypted_headers::decrypt_headers as encrypted_headers_decrypt;
#[cfg(feature = "encryption")]
pub use encryption::{
    BuiltRequest, EncryptionError, EphemeralContext, RequestBuilder, ServiceKeyPair,
    ServiceRecipient,
};

pub use bytes::Bytes;
pub use nats_micro_macros::{service, service_error, service_handlers};

#[doc(hidden)]
pub mod __test_support {
    #[cfg(feature = "encryption")]
    pub fn prepare_request_for_dispatch_with_state(
        state: &crate::StateMap,
        req: crate::NatsRequest,
    ) -> Result<(crate::NatsRequest, Option<[u8; 32]>), crate::NatsErrorResponse> {
        crate::app::NatsApp::prepare_request_for_dispatch_with_state(state, req)
    }
}

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
