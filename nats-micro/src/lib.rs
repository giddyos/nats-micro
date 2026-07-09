//! NATS microservice framework utilities.
//!
//! # Dependency ownership
//!
//! `nats-micro` owns `async-nats` and `thiserror` for normal usage.
//!
//! You do not need to add `async-nats` to construct generated clients or run a
//! [`NatsApp`]. Use [`NatsClient`] or [`async_nats`] for raw NATS APIs.
//!
//! You do not need to add `thiserror` to define service errors. Use
//! [`service_error`].
//!
//! # Service errors
//!
//! ```rust
//! use nats_micro::service_error;
//!
//! #[service_error]
//! pub enum CreateUserError {
//!     #[code(409)]
//!     #[error("email {0} already exists")]
//!     EmailExists(String),
//!
//!     #[error("database query failed")]
//!     Database(#[from] std::io::Error),
//! }
//! ```
//!
//! Raw `thiserror` is available as [`thiserror`] / [`Error`] for advanced local
//! error types:
//!
//! ```rust,ignore
//! #[derive(Debug, nats_micro::Error)]
//! pub enum LocalError {
//!     #[error("bad config: {0}")]
//!     BadConfig(String),
//! }
//! ```
//!
//! Because `thiserror`'s generated code refers to a crate named `thiserror`
//! internally, raw use through a re-export can still require a direct
//! `thiserror` dependency or a crate alias depending on Rust/`thiserror`
//! behavior. Prefer [`service_error`] for service errors.

mod app;
mod auth;
#[cfg(feature = "client")]
mod client;
mod consumer;
#[cfg(feature = "encryption")]
pub mod encryption;
mod error;
mod extractors;
mod handler;
#[cfg(feature = "napi")]
mod napi_support;
pub mod prelude;
mod registry;
mod request;
mod response;
mod service;
mod shutdown_signal;
mod state;
mod utils;

pub use anyhow;
pub use nats_micro_shared::{FrameworkError, TransportError};
pub use thiserror;
pub use thiserror::Error;
pub use thiserror::Error as ThisError;

pub use app::{HandlerPanicPolicy, NatsApp, NatsAppConfig, WorkerFailurePolicy};
pub use async_nats;
pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
pub type NatsClient = async_nats::Client;
pub type NatsMessage = async_nats::Message;
pub type NatsHeaderMap = async_nats::HeaderMap;
pub type NatsHeaderName = async_nats::HeaderName;
pub type NatsHeaderValue = async_nats::HeaderValue;
pub type NatsConsumerConfig = async_nats::jetstream::consumer::push::Config;
pub use auth::{Auth, AuthError, FromAuthRequest};
pub use consumer::{ConsumerDefinition, ConsumerHandlerFn};
pub use error::{
    ClientError, ClientTransportError, FromNatsErrorResponse, IntoNatsError, NatsError,
    NatsErrorResponse, ServiceErrorMatch,
};
pub use extractors::{
    FromPayload, FromRequest, FromSubjectParam, IntoPayloadInner, Json, Payload, Proto, RequestId,
    State, Subject, SubjectParam,
};
pub use handler::{HandlerFn, RequestContext};
#[cfg(feature = "napi")]
pub use napi;
#[cfg(feature = "napi")]
pub use napi_derive;
pub use prost;
pub use request::{Header, Headers, NatsRequest};
pub use response::{IntoNatsResponse, NatsResponse, X_SUCCESS_HEADER};
pub use serde;
pub use serde_json;
pub use service::{
    AuthPolicy, ConsumerInfo, EndpointDefinition, EndpointDescriptor, EndpointInfo, NatsService,
    ParamInfo, PayloadEncoding, PayloadMeta, ResponseEncoding, ResponseMeta, ServiceContract,
    ServiceDefinition, ServiceMetadata,
};
pub use shutdown_signal::ShutdownSignal;
pub use state::StateMap;

#[cfg(feature = "client")]
pub use client::{
    AuthOptions, ClientCallOptions, ConnectOptions, ConnectedClient, X_CLIENT_VERSION_HEADER,
    connect,
};

#[cfg(feature = "encryption")]
pub use encryption::{
    BuiltRequest, Encrypted, EncryptionError, EphemeralContext, RequestBuilder, ServiceKeyPair,
    ServiceRecipient, decrypt_headers as encrypted_headers_decrypt,
};

pub use bytes::Bytes;
#[cfg(feature = "napi")]
pub use nats_micro_macros::object;
pub use nats_micro_macros::{service, service_error, service_handlers};

#[cfg(feature = "napi")]
#[doc(hidden)]
pub mod __private {
    #[doc(hidden)]
    pub trait NapiObject {}

    #[doc(hidden)]
    pub trait NapiServiceError {}

    #[doc(hidden)]
    #[inline(always)]
    pub fn assert_napi_object<T: NapiObject>() {}

    #[doc(hidden)]
    #[inline(always)]
    pub fn assert_napi_service_error<T: NapiServiceError>() {}

    impl NapiObject for String {}
    impl NapiObject for bool {}
    impl NapiObject for i8 {}
    impl NapiObject for i16 {}
    impl NapiObject for i32 {}
    impl NapiObject for i64 {}
    impl NapiObject for isize {}
    impl NapiObject for u8 {}
    impl NapiObject for u16 {}
    impl NapiObject for u32 {}
    impl NapiObject for u64 {}
    impl NapiObject for usize {}
    impl NapiObject for f32 {}
    impl NapiObject for f64 {}

    impl<T: NapiObject> NapiObject for Option<T> {}
    impl<T: NapiObject> NapiObject for Vec<T> {}
}

#[doc(hidden)]
pub mod __thiserror {
    pub use crate::thiserror::*;
}

#[cfg(feature = "napi")]
#[doc(hidden)]
pub mod __napi {
    #[doc(hidden)]
    pub use crate::error::NapiClientError;
    #[doc(hidden)]
    pub use crate::napi_support::{NapiClientHeaderValue, client_call_options_from_headers};
    #[doc(hidden)]
    pub use crate::{AuthOptions, ConnectOptions, ConnectedClient, connect};
}

#[doc(hidden)]
pub mod __test_support {

    #[doc(hidden)]
    pub use crate::app::success_headers;

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub fn prepare_request_for_dispatch_with_state(
        state: &crate::StateMap,
        req: crate::NatsRequest,
    ) -> Result<(crate::NatsRequest, Option<[u8; 32]>), crate::NatsErrorResponse> {
        let prepared = crate::app::NatsApp::prepare_request_for_dispatch_with_state(state, req)?;
        Ok((prepared.request, prepared.ephemeral_pub))
    }
}

#[doc(hidden)]
pub mod __macros {
    pub use crate::NatsConsumerConfig as ConsumerConfig;
    pub use crate::error::FromNatsErrorResponse;
    pub use crate::error::IntoNatsError;
    pub use crate::error::ServiceErrorMatch;
    pub use crate::extractors::FromRequest;
    pub use crate::handler::into_handler_fn;
    pub use crate::handler::{HandlerFuture, RequestContext};
    pub use crate::registry::ServiceRegistration;
    pub use crate::response::{IntoNatsResponse, NatsResponse};
    pub use crate::service::{
        AuthPolicy, ConsumerInfo, EndpointInfo, NatsService, ParamInfo, PayloadEncoding,
        PayloadMeta, ResponseEncoding, ResponseMeta, ServiceDefinition, build_subject,
    };
    pub use inventory;

    // Error helpers
    pub use crate::error::deserialize_error_response;
    pub use crate::error::deserialize_error_response_payload;
    pub use crate::error::invalid_response;
    pub use crate::error::try_deserialize_error_response;

    // Response / client helpers
    pub use crate::response::deserialize_optional_proto_response;
    pub use crate::response::deserialize_optional_response;
    pub use crate::response::deserialize_optional_unit_response;
    pub use crate::response::deserialize_proto_response;
    pub use crate::response::deserialize_response;
    pub use crate::response::deserialize_unit_response;
    pub use crate::response::optional_response_from_headers;
    pub use crate::response::raw_response_to_bytes;
    pub use crate::response::raw_response_to_optional_bytes;
    pub use crate::response::raw_response_to_optional_string;
    pub use crate::response::raw_response_to_string;
    pub use crate::response::response_success_from_headers;
    pub use crate::response::serialize_proto_payload;
    pub use crate::response::serialize_serde_payload;

    #[cfg(feature = "encryption")]
    pub use crate::response::decrypt_client_response;
}
