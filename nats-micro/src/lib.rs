//! Static, typed NATS microservices.
//!
//! `#[service]` generates concrete dispatch, client, contract, and startup
//! code. Runtime requests borrow from their transport frame and do not pass
//! through a registry, erased handler, or type map.

mod app;
mod auth;
mod client;
pub mod codec;
mod consumer;
mod contract;
mod deployment;
#[cfg(feature = "encryption")]
pub mod encryption;
mod error;
mod extractors;
mod handler;
mod message;
pub mod prelude;
mod request;
mod response;
pub mod runtime;
mod service;
mod shutdown_signal;
mod spec;
mod state_ref;
pub mod subject;
#[cfg(feature = "telemetry")]
mod telemetry;
#[cfg(feature = "test-util")]
pub mod testing;
mod trace;

pub use anyhow;
pub use async_nats;
pub use bytes::Bytes;
pub use nats_micro_shared::{FrameworkError, TransportError};
#[cfg(feature = "protobuf")]
pub use prost;
pub use serde;
pub use serde_json;
pub use thiserror;
pub use thiserror::Error;
pub use thiserror::Error as ThisError;
pub use tokio;

pub type Result<T> = anyhow::Result<T>;
pub type NatsClient = async_nats::Client;
pub type NatsMessage = async_nats::Message;
pub type NatsHeaderMap = async_nats::HeaderMap;
pub type NatsHeaderName = async_nats::HeaderName;
pub type NatsHeaderValue = async_nats::HeaderValue;

#[cfg(feature = "encryption")]
pub use app::EncryptedService;
pub use app::{
    App, AppConfig, ConnectionConfig, Cons, HandlerPanicPolicy, Nil, Profile, RunStartupHook,
    RunningApp, Runtime, Service, ServiceSet, ServiceSetValidator, StartError, TelemetryConfig,
    WorkerFailurePolicy, assert_services_compatible, str_eq, validate_service_set,
};
pub use auth::{Auth, AuthError, FromRequestMeta};
pub use client::{
    AuthOptions, BytesDecoder, ClientBuildError, ClientRequest, ClientResponse,
    ClientResponseDecoder, ClientTransport, ConnectOptions, ConnectedClient, EmptyDecoder,
    NatsTransport, OptionalBytesDecoder, OptionalTextDecoder, OptionalVecDecoder, PublishCall,
    RequestCall, ResponseDecoder, Subject as ClientSubject, TextDecoder, VecDecoder,
    X_CLIENT_VERSION_HEADER, connect, merge_headers,
};
#[cfg(feature = "encryption")]
pub use client::{EncryptedPublishCall, EncryptedRequestCall};
#[cfg(feature = "json")]
pub use client::{JsonDecoder, OptionalJsonDecoder};
#[cfg(feature = "protobuf")]
pub use client::{OptionalProtoDecoder, ProtoDecoder};
pub use codec::decode_text;
#[cfg(feature = "json")]
pub use codec::{decode_json, encode_json};
#[cfg(feature = "protobuf")]
pub use codec::{decode_proto, encode_proto};
pub use consumer::{ConsumerAction, ConsumerHandler};
pub use contract::{
    ContractConsumer, ContractDocument, ContractOperation, ContractParam, ContractService,
    ContractValidationError,
};
pub use deployment::{
    ConsumerDeployment, DeploymentFormat, DeploymentMetadata, DeploymentOptions,
    EndpointDeployment, EnvironmentVariable, ReadinessConfiguration, SecretPlaceholder,
};
pub use error::{
    ClientError, ClientTransportError, FromNatsErrorResponse, IntoNatsError, NatsError,
    NatsErrorResponse, ServiceErrorMatch,
};
pub use extractors::{Json, Proto};
#[cfg(feature = "encryption")]
pub use handler::EncryptedRequestEndpoint;
pub use handler::{DispatchResult, RequestEndpoint, SubscriptionHandler};
pub use message::JsonMessage;
#[cfg(feature = "napi")]
pub use napi;
#[cfg(feature = "napi")]
pub use napi_derive;
pub use request::{Body, Headers, Request, RequestId, RequestMeta, Text};
pub use response::{ErrorReply, IntoServiceError, PRESENT_HEADER, Response};
pub use service::{LocalService, OperationMarker, PublishOperation, StaticService};
pub use shutdown_signal::ShutdownState;
pub use spec::{
    AuthPolicy, Codec, ConsumerSpec, OperationKind, OperationSpec, ParamSpec, ServiceContract,
    ServiceSpec,
};
pub use state_ref::{FromAppState, StateRef};
pub use subject::{FromSubject, push_subject_param, segment, subject_matches, subject_param_len};
#[cfg(feature = "telemetry")]
pub use telemetry::{MetricEvent, MetricName, TelemetryLayer};

#[cfg(feature = "encryption")]
pub use encryption::{
    BuiltRequest, Encrypted, EncryptedRequest, EncryptionError, EphemeralContext, RequestBuilder,
    ServiceKeyPair, ServiceRecipient, decrypt_headers as encrypted_headers_decrypt,
};

#[cfg(all(feature = "macros", feature = "napi"))]
pub use nats_micro_macros::object;
#[cfg(feature = "macros")]
pub use nats_micro_macros::{
    AppState, application, live_test, main, message, service, service_error, test,
};

#[cfg(feature = "napi")]
#[doc(hidden)]
pub mod __private {
    pub trait NapiObject {}
    pub trait NapiServiceError {}

    #[inline(always)]
    pub fn assert_napi_object<T: NapiObject>() {}

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
