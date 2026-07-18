use std::{marker::PhantomData, str::FromStr, time::Duration};

use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro_shared::{FrameworkError, TransportError};
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::{ClientRequest, ClientResponse, ClientTransport, Subject};
use crate::{ClientError, FromNatsErrorResponse, NatsErrorResponse};

type RequestCallMarker<R, E, D> = fn() -> (R, E, D);

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ClientBuildError {
    message: String,
}

impl ClientBuildError {
    fn header(name: &str, error: impl std::fmt::Display) -> Self {
        Self {
            message: format!("invalid header `{name}`: {error}"),
        }
    }
}

#[must_use]
pub fn merge_headers(
    defaults: Option<&HeaderMap>,
    per_call: Option<HeaderMap>,
) -> Option<HeaderMap> {
    match (defaults, per_call) {
        (None, None) => None,
        (None, Some(headers)) => Some(headers),
        (Some(defaults), None) => Some(defaults.clone()),
        (Some(defaults), Some(headers)) => {
            let mut merged = defaults.clone();
            for (name, values) in headers.iter() {
                for (index, value) in values.iter().enumerate() {
                    if index == 0 {
                        merged.insert(name.clone(), value.clone());
                    } else {
                        merged.append(name.clone(), value.clone());
                    }
                }
            }
            Some(merged)
        }
    }
}

pub trait ResponseDecoder<R, E>
where
    E: std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<R, ClientError<E>>;
}

#[derive(Debug, Clone, Copy)]
pub struct JsonDecoder;
#[derive(Debug, Clone, Copy)]
pub struct OptionalJsonDecoder;
#[derive(Debug, Clone, Copy)]
pub struct ProtoDecoder;
#[derive(Debug, Clone, Copy)]
pub struct OptionalProtoDecoder;
#[derive(Debug, Clone, Copy)]
pub struct TextDecoder;
#[derive(Debug, Clone, Copy)]
pub struct OptionalTextDecoder;
#[derive(Debug, Clone, Copy)]
pub struct BytesDecoder;
#[derive(Debug, Clone, Copy)]
pub struct OptionalBytesDecoder;
#[derive(Debug, Clone, Copy)]
pub struct VecDecoder;
#[derive(Debug, Clone, Copy)]
pub struct OptionalVecDecoder;
#[derive(Debug, Clone, Copy)]
pub struct EmptyDecoder;
#[derive(Debug, Clone, Copy)]
pub struct ClientResponseDecoder;

fn error_response<E>(response: &ClientResponse) -> Result<Option<ClientError<E>>, ClientError<E>>
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    let Some(headers) = response.headers.as_ref() else {
        return Ok(None);
    };
    let reported_error = headers.get("Nats-Service-Error-Code").is_some()
        || headers.get("Nats-Micro-Error-Kind").is_some();
    if !reported_error {
        return Ok(None);
    }
    let error =
        serde_json::from_slice::<NatsErrorResponse>(&response.payload).map_err(|error| {
            ClientError::invalid_response(NatsErrorResponse::framework(
                FrameworkError::InvalidResponse,
                format!("service error headers accompanied malformed JSON: {error}"),
            ))
        })?;
    Ok(Some(ClientError::from_service_response(error)))
}

fn ensure_success<E>(response: &ClientResponse) -> Result<(), ClientError<E>>
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    if let Some(error) = error_response(response)? {
        Err(error)
    } else {
        Ok(())
    }
}

fn is_optional_none(response: &ClientResponse) -> bool {
    response
        .headers
        .as_ref()
        .and_then(|headers| headers.get(crate::PRESENT_HEADER))
        .is_some_and(|value| value.as_str() == "0")
}

fn deserialize_error<E>(error: impl std::fmt::Display) -> ClientError<E>
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    ClientError::deserialize(NatsErrorResponse::framework(
        FrameworkError::DeserializationError,
        error.to_string(),
    ))
}

impl<R, E> ResponseDecoder<R, E> for JsonDecoder
where
    R: DeserializeOwned,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<R, ClientError<E>> {
        ensure_success(&response)?;
        serde_json::from_slice(&response.payload).map_err(deserialize_error)
    }
}

impl<R, E> ResponseDecoder<Option<R>, E> for OptionalJsonDecoder
where
    R: DeserializeOwned,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Option<R>, ClientError<E>> {
        ensure_success(&response)?;
        if is_optional_none(&response) {
            Ok(None)
        } else {
            serde_json::from_slice(&response.payload)
                .map(Some)
                .map_err(deserialize_error)
        }
    }
}

impl<R, E> ResponseDecoder<R, E> for ProtoDecoder
where
    R: prost::Message + Default,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<R, ClientError<E>> {
        ensure_success(&response)?;
        R::decode(response.payload).map_err(deserialize_error)
    }
}

impl<R, E> ResponseDecoder<Option<R>, E> for OptionalProtoDecoder
where
    R: prost::Message + Default,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Option<R>, ClientError<E>> {
        ensure_success(&response)?;
        if is_optional_none(&response) {
            Ok(None)
        } else {
            R::decode(response.payload)
                .map(Some)
                .map_err(deserialize_error)
        }
    }
}

impl<E> ResponseDecoder<String, E> for TextDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<String, ClientError<E>> {
        ensure_success(&response)?;
        String::from_utf8(response.payload.to_vec()).map_err(deserialize_error)
    }
}

impl<E> ResponseDecoder<Option<String>, E> for OptionalTextDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Option<String>, ClientError<E>> {
        ensure_success(&response)?;
        if is_optional_none(&response) {
            Ok(None)
        } else {
            String::from_utf8(response.payload.to_vec())
                .map(Some)
                .map_err(deserialize_error)
        }
    }
}

impl<E> ResponseDecoder<Bytes, E> for BytesDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Bytes, ClientError<E>> {
        ensure_success(&response)?;
        Ok(response.payload)
    }
}

impl<E> ResponseDecoder<Option<Bytes>, E> for OptionalBytesDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Option<Bytes>, ClientError<E>> {
        ensure_success(&response)?;
        Ok((!is_optional_none(&response)).then_some(response.payload))
    }
}

impl<E> ResponseDecoder<Vec<u8>, E> for VecDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Vec<u8>, ClientError<E>> {
        ensure_success(&response)?;
        Ok(response.payload.to_vec())
    }
}

impl<E> ResponseDecoder<Option<Vec<u8>>, E> for OptionalVecDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<Option<Vec<u8>>, ClientError<E>> {
        ensure_success(&response)?;
        Ok((!is_optional_none(&response)).then(|| response.payload.to_vec()))
    }
}

impl<E> ResponseDecoder<(), E> for EmptyDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<(), ClientError<E>> {
        ensure_success(&response)
    }
}

impl<E> ResponseDecoder<ClientResponse, E> for ClientResponseDecoder
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    fn decode(response: ClientResponse) -> Result<ClientResponse, ClientError<E>> {
        ensure_success(&response)?;
        Ok(response)
    }
}

pub struct RequestCall<'a, T, R, E, D> {
    transport: &'a T,
    subject: Subject<'a>,
    payload: Result<Bytes, crate::ErrorReply>,
    default_headers: Option<&'a HeaderMap>,
    headers: Option<HeaderMap>,
    timeout: Option<Duration>,
    marker: PhantomData<RequestCallMarker<R, E, D>>,
}

impl<'a, T, R, E, D> RequestCall<'a, T, R, E, D> {
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        transport: &'a T,
        subject: Subject<'a>,
        payload: Result<Bytes, crate::ErrorReply>,
        default_headers: Option<&'a HeaderMap>,
        timeout: Option<Duration>,
    ) -> Self {
        Self {
            transport,
            subject,
            payload,
            default_headers,
            headers: None,
            timeout,
            marker: PhantomData,
        }
    }

    pub fn header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, ClientBuildError> {
        let name = name.as_ref();
        let header_name = async_nats::HeaderName::from_str(name)
            .map_err(|error| ClientBuildError::header(name, error))?;
        let header_value = value
            .as_ref()
            .parse::<async_nats::HeaderValue>()
            .map_err(|error| ClientBuildError::header(name, error))?;
        self.headers
            .get_or_insert_with(HeaderMap::new)
            .insert(header_name, header_value);
        Ok(self)
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<T, R, E, D> RequestCall<'_, T, R, E, D>
where
    T: ClientTransport,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
    D: ResponseDecoder<R, E>,
{
    pub async fn send(self) -> Result<R, ClientError<E>> {
        let payload = self.payload.map_err(|error| {
            ClientError::serialize(NatsErrorResponse::new(
                error.code,
                error.kind,
                error.message,
                error.request_id.unwrap_or_default(),
            ))
        })?;
        let response = self
            .transport
            .request(ClientRequest {
                subject: self.subject,
                payload,
                headers: merge_headers(self.default_headers, self.headers),
                timeout: self.timeout,
            })
            .await
            .map_err(transport_client_error)?;
        D::decode(response)
    }
}

pub struct PublishCall<'a, T, E> {
    transport: &'a T,
    subject: Subject<'a>,
    payload: Result<Bytes, crate::ErrorReply>,
    default_headers: Option<&'a HeaderMap>,
    headers: Option<HeaderMap>,
    marker: PhantomData<fn() -> E>,
}

impl<'a, T, E> PublishCall<'a, T, E> {
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        transport: &'a T,
        subject: Subject<'a>,
        payload: Result<Bytes, crate::ErrorReply>,
        default_headers: Option<&'a HeaderMap>,
    ) -> Self {
        Self {
            transport,
            subject,
            payload,
            default_headers,
            headers: None,
            marker: PhantomData,
        }
    }

    pub fn header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, ClientBuildError> {
        let name = name.as_ref();
        let header_name = async_nats::HeaderName::from_str(name)
            .map_err(|error| ClientBuildError::header(name, error))?;
        let header_value = value
            .as_ref()
            .parse::<async_nats::HeaderValue>()
            .map_err(|error| ClientBuildError::header(name, error))?;
        self.headers
            .get_or_insert_with(HeaderMap::new)
            .insert(header_name, header_value);
        Ok(self)
    }
}

impl<T, E> PublishCall<'_, T, E>
where
    T: ClientTransport,
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    pub async fn send(self) -> Result<(), ClientError<E>> {
        let payload = self.payload.map_err(|error| {
            ClientError::serialize(NatsErrorResponse::new(
                error.code,
                error.kind,
                error.message,
                error.request_id.unwrap_or_default(),
            ))
        })?;
        self.transport
            .publish(
                self.subject,
                payload,
                merge_headers(self.default_headers, self.headers),
            )
            .await
            .map_err(transport_client_error)
    }
}

fn transport_client_error<E>(error: TransportError) -> ClientError<E>
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    ClientError::request(NatsErrorResponse::transport(
        error,
        "NATS client transport operation failed",
    ))
}
