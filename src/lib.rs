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
pub use error::{
    ClientError, ClientTransportError, FromNatsErrorResponse, IntoNatsError, NatsError,
    NatsErrorResponse, ServiceErrorMatch,
};
pub use extractors::{
    FromPayload, FromRequest, FromSubjectParam, Json, Payload, Proto, RequestId, State, Subject,
    SubjectParam,
};
pub use handler::{HandlerFn, RequestContext};
pub use request::{Header, Headers, NatsRequest};
pub use response::{IntoNatsResponse, NatsResponse, X_SUCCESS_HEADER};
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

    fn deserialize_error_response_payload(payload: &[u8]) -> Option<crate::NatsErrorResponse> {
        ::serde_json::from_slice::<crate::NatsErrorResponse>(payload).ok()
    }

    fn invalid_response(message: impl Into<String>) -> crate::NatsErrorResponse {
        crate::NatsErrorResponse::internal("INVALID_RESPONSE", message.into())
    }

    fn response_success_from_headers<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
    ) -> Result<Option<bool>, crate::ClientError<E>> {
        let Some(headers) = headers else {
            return Ok(None);
        };
        let Some(value) = headers.get(crate::X_SUCCESS_HEADER) else {
            return Ok(None);
        };

        let value = value.as_str();
        if value.eq_ignore_ascii_case("true") {
            Ok(Some(true))
        } else if value.eq_ignore_ascii_case("false") {
            Ok(Some(false))
        } else {
            Err(crate::ClientError::invalid_response(invalid_response(
                format!("invalid {} header value: {value}", crate::X_SUCCESS_HEADER,),
            )))
        }
    }

    fn deserialize_error_response<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        payload: &[u8],
    ) -> Result<crate::NatsErrorResponse, crate::ClientError<E>> {
        deserialize_error_response_payload(payload).ok_or_else(|| {
            crate::ClientError::invalid_response(invalid_response(format!(
                "expected serialized NatsErrorResponse payload when {}=false",
                crate::X_SUCCESS_HEADER,
            )))
        })
    }

    pub fn try_deserialize_error_response(payload: &[u8]) -> Option<crate::NatsErrorResponse> {
        deserialize_error_response_payload(payload).filter(|error| error.code >= 400)
    }

    pub fn deserialize_response<
        T: ::serde::de::DeserializeOwned,
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        payload: &[u8],
    ) -> Result<T, crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(true) => {
                return ::serde_json::from_slice(payload).map_err(|error| {
                    crate::ClientError::deserialize(crate::NatsErrorResponse::internal(
                        "DESERIALIZATION_ERROR",
                        error.to_string(),
                    ))
                });
            }
            Some(false) => {
                let response = deserialize_error_response::<E>(payload)?;
                return Err(crate::ClientError::from_service_response(response));
            }
            None => {}
        }

        match ::serde_json::from_slice(payload) {
            Ok(value) => Ok(value),
            Err(error) => {
                if let Some(response) = try_deserialize_error_response(payload) {
                    Err(crate::ClientError::from_service_response(response))
                } else {
                    Err(crate::ClientError::deserialize(
                        crate::NatsErrorResponse::internal(
                            "DESERIALIZATION_ERROR",
                            error.to_string(),
                        ),
                    ))
                }
            }
        }
    }

    pub fn deserialize_proto_response<
        T: ::prost::Message + Default,
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        payload: &[u8],
    ) -> Result<T, crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(true) => {
                return T::decode(payload).map_err(|error| {
                    crate::ClientError::deserialize(crate::NatsErrorResponse::internal(
                        "DESERIALIZATION_ERROR",
                        error.to_string(),
                    ))
                });
            }
            Some(false) => {
                let response = deserialize_error_response::<E>(payload)?;
                return Err(crate::ClientError::from_service_response(response));
            }
            None => {}
        }

        match T::decode(payload) {
            Ok(value) => Ok(value),
            Err(error) => {
                if let Some(response) = try_deserialize_error_response(payload) {
                    Err(crate::ClientError::from_service_response(response))
                } else {
                    Err(crate::ClientError::deserialize(
                        crate::NatsErrorResponse::internal(
                            "DESERIALIZATION_ERROR",
                            error.to_string(),
                        ),
                    ))
                }
            }
        }
    }

    pub fn deserialize_unit_response<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        payload: &[u8],
    ) -> Result<(), crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(true) => {
                if payload.is_empty() {
                    return Ok(());
                }
                return Err(crate::ClientError::invalid_response(invalid_response(
                    "expected empty response payload when x-success=true",
                )));
            }
            Some(false) => {
                let response = deserialize_error_response::<E>(payload)?;
                return Err(crate::ClientError::from_service_response(response));
            }
            None => {}
        }

        if payload.is_empty() {
            return Ok(());
        }
        if let Some(response) = try_deserialize_error_response(payload) {
            return Err(crate::ClientError::from_service_response(response));
        }
        Err(crate::ClientError::invalid_response(
            crate::NatsErrorResponse::internal(
                "DESERIALIZATION_ERROR",
                "expected empty response payload",
            ),
        ))
    }

    pub fn raw_response_to_string<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        payload: &[u8],
    ) -> Result<String, crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(true) => {}
            Some(false) => {
                let response = deserialize_error_response::<E>(payload)?;
                return Err(crate::ClientError::from_service_response(response));
            }
            None => {
                if let Some(response) = try_deserialize_error_response(payload) {
                    return Err(crate::ClientError::from_service_response(response));
                }
            }
        }

        String::from_utf8(payload.to_vec()).map_err(|error| {
            crate::ClientError::deserialize(crate::NatsErrorResponse::internal(
                "DESERIALIZATION_ERROR",
                error.to_string(),
            ))
        })
    }

    pub fn raw_response_to_bytes<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        payload: &[u8],
    ) -> Result<Vec<u8>, crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(true) => {}
            Some(false) => {
                let response = deserialize_error_response::<E>(payload)?;
                return Err(crate::ClientError::from_service_response(response));
            }
            None => {
                if let Some(response) = try_deserialize_error_response(payload) {
                    return Err(crate::ClientError::from_service_response(response));
                }
            }
        }
        Ok(payload.to_vec())
    }

    #[cfg(feature = "encryption")]
    pub fn decrypt_client_response<
        E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
    >(
        headers: Option<&crate::async_nats::HeaderMap>,
        eph_ctx: &crate::EphemeralContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, crate::ClientError<E>> {
        match response_success_from_headers::<E>(headers)? {
            Some(false) => return Ok(payload.to_vec()),
            Some(true) => {
                return eph_ctx.decrypt_response(payload).map_err(|error| {
                    crate::ClientError::decrypt(crate::NatsErrorResponse::internal(
                        "DECRYPT_ERROR",
                        error.to_string(),
                    ))
                });
            }
            None => {}
        }

        match eph_ctx.decrypt_response(payload) {
            Ok(plaintext) => Ok(plaintext),
            Err(error) => {
                if let Some(response) = try_deserialize_error_response(payload) {
                    Err(crate::ClientError::from_service_response(response))
                } else {
                    Err(crate::ClientError::decrypt(
                        crate::NatsErrorResponse::internal("DECRYPT_ERROR", error.to_string()),
                    ))
                }
            }
        }
    }

    pub fn serialize_serde_payload<T: ::serde::Serialize>(
        payload: &T,
    ) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
        ::serde_json::to_vec(payload)
            .map(::bytes::Bytes::from)
            .map_err(|e| crate::NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string()))
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
