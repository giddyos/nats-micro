pub use crate::Bytes;
pub use crate::ConsumerConfig;
pub use crate::ShutdownSignal;
pub use crate::auth::{Auth, AuthError, FromAuthRequest};
pub use crate::consumer::{ConsumerDefinition, ConsumerHandlerFn};
pub use crate::error::{ClientError, IntoNatsError, NatsError, NatsErrorResponse};
pub use crate::extractors::{
    FromPayload, FromRequest, FromSubjectParam, IntoPayloadInner, Json, Payload, Proto, RequestId,
    State, Subject, SubjectParam,
};
pub use crate::handler::{HandlerFn, RequestContext};
pub use crate::request::NatsRequest;
pub use crate::request::{Header, Headers};
pub use crate::response::{IntoNatsResponse, NatsResponse};
pub use crate::service::{
    ConsumerInfo, EndpointDefinition, EndpointInfo, NatsService, ParamInfo, ServiceDefinition,
    ServiceMetadata,
};
pub use crate::state::StateMap;
pub use crate::{NatsApp, NatsAppConfig, WorkerFailurePolicy};

#[cfg(feature = "client")]
pub use crate::{ClientCallOptions, X_CLIENT_VERSION_HEADER};

#[cfg(feature = "encryption")]
pub use crate::{
    BuiltRequest, Encrypted, EncryptionError, EphemeralContext, RequestBuilder, ServiceKeyPair,
    ServiceRecipient,
};

#[cfg(feature = "napi")]
pub use crate::{napi, napi_derive};

#[cfg(feature = "napi")]
pub use nats_micro_macros::object;

pub use crate::async_nats;
pub use nats_micro_macros::{service, service_error, service_handlers};

pub use thiserror::Error as ServiceError;
