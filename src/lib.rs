#![allow(unused)]

mod app;
mod auth;
#[cfg(feature = "client")]
mod client;
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
pub use async_nats;
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
    ConsumerInfo, EndpointDefinition, EndpointInfo, NatsService, ParamInfo, PayloadEncoding,
    PayloadMeta, ResponseEncoding, ResponseMeta, ServiceDefinition, ServiceMetadata,
};
pub use state::StateMap;

#[cfg(feature = "client")]
pub use client::ClientCallOptions;

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
        ConsumerInfo, EndpointInfo, NatsService, ParamInfo, PayloadEncoding, PayloadMeta,
        ResponseEncoding, ResponseMeta, ServiceDefinition,
    };
    pub use async_nats::jetstream::consumer::push::Config as ConsumerConfig;
    pub use inventory;

    pub fn deserialize_response<T: ::serde::de::DeserializeOwned>(
        payload: &[u8],
    ) -> Result<T, crate::NatsErrorResponse> {
        if let Ok(err) = ::serde_json::from_slice::<crate::NatsErrorResponse>(payload) {
            if err.code >= 400 {
                return Err(err);
            }
        }
        ::serde_json::from_slice(payload).map_err(|e| {
            crate::NatsErrorResponse::internal("DESERIALIZATION_ERROR", e.to_string())
        })
    }

    pub fn deserialize_proto_response<T: ::prost::Message + Default>(
        payload: &[u8],
    ) -> Result<T, crate::NatsErrorResponse> {
        if let Ok(err) = ::serde_json::from_slice::<crate::NatsErrorResponse>(payload) {
            if err.code >= 400 {
                return Err(err);
            }
        }
        T::decode(payload).map_err(|e| {
            crate::NatsErrorResponse::internal("DESERIALIZATION_ERROR", e.to_string())
        })
    }

    pub fn raw_response_to_string(payload: &[u8]) -> Result<String, crate::NatsErrorResponse> {
        if let Ok(err) = ::serde_json::from_slice::<crate::NatsErrorResponse>(payload) {
            if err.code >= 400 {
                return Err(err);
            }
        }
        String::from_utf8(payload.to_vec()).map_err(|e| {
            crate::NatsErrorResponse::internal("DESERIALIZATION_ERROR", e.to_string())
        })
    }

    pub fn raw_response_to_bytes(payload: &[u8]) -> Result<Vec<u8>, crate::NatsErrorResponse> {
        if let Ok(err) = ::serde_json::from_slice::<crate::NatsErrorResponse>(payload) {
            if err.code >= 400 {
                return Err(err);
            }
        }
        Ok(payload.to_vec())
    }

    pub fn serialize_serde_payload<T: ::serde::Serialize>(
        payload: &T,
    ) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
        ::serde_json::to_vec(payload)
            .map(::bytes::Bytes::from)
            .map_err(|e| {
                crate::NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string())
            })
    }

    pub fn serialize_proto_payload<T: ::prost::Message>(
        payload: &T,
    ) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
        let mut buf = Vec::new();
        payload.encode(&mut buf).map_err(|e| {
            crate::NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string())
        })?;
        Ok(::bytes::Bytes::from(buf))
    }
}
