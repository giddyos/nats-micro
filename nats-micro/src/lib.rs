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

pub use app::{NatsApp, NatsAppConfig, WorkerFailurePolicy};
pub use async_nats;
pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
pub use auth::{Auth, AuthError, FromAuthRequest};
pub use consumer::{ConsumerDefinition, ConsumerHandlerFn};
pub use error::{
    ClientError, ClientTransportError, FromNatsErrorResponse, IntoNatsError, NatsError,
    NatsErrorResponse, ServiceErrorMatch,
};
pub use extractors::{
    FromPayload, FromRequest, FromSubjectParam, Json, Payload, Proto, RequestId, State, Subject,
    SubjectParam,
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
    ConsumerInfo, EndpointDefinition, EndpointInfo, NatsService, ParamInfo, PayloadEncoding,
    PayloadMeta, ResponseEncoding, ResponseMeta, ServiceDefinition, ServiceMetadata,
};
pub use shutdown_signal::ShutdownSignal;
pub use state::StateMap;

#[cfg(feature = "client")]
pub use client::{ClientCallOptions, X_CLIENT_VERSION_HEADER};

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

#[cfg(feature = "napi")]
#[doc(hidden)]
pub mod __napi {
    pub use crate::error::NapiClientError;
    pub use crate::napi_support::{
        ConnectedClient, NapiAuthOptions, NapiClientHeaderValue, NapiConnectOptions,
        client_call_options_from_headers, connect,
    };
}

#[doc(hidden)]
pub mod __test_support {

    pub use crate::app::success_headers;

    #[cfg(feature = "encryption")]
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
    pub use crate::error::FromNatsErrorResponse;
    pub use crate::error::IntoNatsError;
    pub use crate::error::ServiceErrorMatch;
    pub use crate::extractors::FromRequest;
    pub use crate::handler::into_handler_fn;
    pub use crate::handler::{HandlerFuture, RequestContext};
    pub use crate::registry::ServiceRegistration;
    pub use crate::response::{IntoNatsResponse, NatsResponse};
    pub use crate::service::{
        ConsumerInfo, EndpointInfo, NatsService, ParamInfo, PayloadEncoding, PayloadMeta,
        ResponseEncoding, ResponseMeta, ServiceDefinition, build_subject,
    };
    pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
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
