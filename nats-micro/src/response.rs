use bytes::Bytes;
use nats_micro_shared::FrameworkError;

use crate::{
    Proto, error::NatsErrorResponse, extractors::Json, handler::RequestContext, serde::Serialize,
};

pub const X_SUCCESS_HEADER: &str = "x-success";
pub const X_OPTIONAL_RESPONSE_HEADER: &str = "x-nats-micro-optional-response";

#[derive(Debug, Clone)]
pub struct NatsResponse {
    pub payload: Bytes,
    pub headers: crate::async_nats::HeaderMap,
}

impl NatsResponse {
    #[must_use]
    pub fn new(payload: impl Into<Bytes>) -> Self {
        Self {
            payload: payload.into(),
            headers: crate::async_nats::HeaderMap::new(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, key: &str, value: impl AsRef<str>) -> Self {
        self.headers.insert(key, value.as_ref());
        self
    }
}

pub trait IntoNatsResponse {
    /// Converts a handler return value into a transport response.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be encoded for transport.
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse>;
}

impl IntoNatsResponse for NatsResponse {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(self)
    }
}

impl IntoNatsResponse for () {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(Bytes::new()))
    }
}

impl IntoNatsResponse for Bytes {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for Vec<u8> {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for String {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for &'static str {
    fn into_response(self, _ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl<T> IntoNatsResponse for Option<T>
where
    T: IntoNatsResponse,
{
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        match self {
            Some(value) => value
                .into_response(ctx)
                .map(|response| response.with_header(X_OPTIONAL_RESPONSE_HEADER, "true")),
            None => Ok(
                NatsResponse::new(Bytes::new()).with_header(X_OPTIONAL_RESPONSE_HEADER, "false")
            ),
        }
    }
}

impl<T> IntoNatsResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        crate::serde_json::to_vec(&self.0)
            .map_err(|e| {
                NatsErrorResponse::framework(
                    FrameworkError::SerializationError,
                    format!("failed to serialize the response payload as JSON: {e}"),
                )
                .with_request_id(ctx.request.request_id.clone())
            })
            .map(NatsResponse::new)
    }
}

impl<T> IntoNatsResponse for Proto<T>
where
    T: crate::prost::Message,
{
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        let mut buf = Vec::new();
        self.0.encode(&mut buf).map_err(|e| {
            NatsErrorResponse::framework(
                FrameworkError::SerializationError,
                format!("failed to encode the response payload as protobuf: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;
        Ok(NatsResponse::new(buf))
    }
}

// Client / response decoding helpers used by generated clients and macros.
/// Reads the explicit success marker from transport headers.
///
/// # Errors
///
/// Returns an invalid-response client error when the header is present but not
/// a valid boolean string.
pub fn response_success_from_headers<
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
        Err(crate::ClientError::invalid_response(
            NatsErrorResponse::framework(
                FrameworkError::InvalidResponse,
                format!(
                    "invalid header `{X_SUCCESS_HEADER}` value `{value}`; expected `true` or `false`"
                ),
            ),
        ))
    }
}

/// Reads the explicit optional-response marker from transport headers.
///
/// # Errors
///
/// Returns an invalid-response client error when the header is present but not
/// a valid boolean string.
pub fn optional_response_from_headers<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
) -> Result<Option<bool>, crate::ClientError<E>> {
    let Some(headers) = headers else {
        return Ok(None);
    };
    let Some(value) = headers.get(X_OPTIONAL_RESPONSE_HEADER) else {
        return Ok(None);
    };

    let value = value.as_str();
    if value.eq_ignore_ascii_case("true") {
        Ok(Some(true))
    } else if value.eq_ignore_ascii_case("false") {
        Ok(Some(false))
    } else {
        Err(crate::ClientError::invalid_response(
            NatsErrorResponse::framework(
                FrameworkError::InvalidResponse,
                format!(
                    "invalid header `{X_OPTIONAL_RESPONSE_HEADER}` value `{value}`; expected `true` or `false`"
                ),
            ),
        ))
    }
}

fn optional_response_is_none<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<bool, crate::ClientError<E>> {
    match optional_response_from_headers::<E>(headers)? {
        Some(false) => Ok(true),
        Some(true) => Ok(false),
        None => Ok(payload.is_empty()),
    }
}

/// Deserializes a JSON response payload into the requested type.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when the payload cannot be decoded as the expected success shape.
pub fn deserialize_response<
    T: crate::serde::de::DeserializeOwned,
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<T, crate::ClientError<E>> {
    match response_success_from_headers::<E>(headers)? {
        Some(true) => {
            return crate::serde_json::from_slice(payload).map_err(|error| {
                crate::ClientError::deserialize(crate::NatsErrorResponse::framework(
                    FrameworkError::DeserializationError,
                    format!("failed to decode a successful response payload as JSON: {error}"),
                ))
            });
        }
        Some(false) => {
            let response = crate::error::deserialize_error_response::<E>(payload)?;
            return Err(crate::ClientError::from_service_response(response));
        }
        None => {}
    }

    match crate::serde_json::from_slice(payload) {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                Err(crate::ClientError::from_service_response(response))
            } else {
                Err(crate::ClientError::deserialize(
                    crate::NatsErrorResponse::framework(
                        FrameworkError::DeserializationError,
                        format!("failed to decode the response payload as JSON: {error}"),
                    ),
                ))
            }
        }
    }
}

/// Deserializes an optional JSON response payload into the requested type.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when a present payload cannot be decoded as the expected success shape.
pub fn deserialize_optional_response<
    T: crate::serde::de::DeserializeOwned,
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Option<T>, crate::ClientError<E>> {
    if let Some(false) = response_success_from_headers::<E>(headers)? {
        let response = crate::error::deserialize_error_response::<E>(payload)?;
        return Err(crate::ClientError::from_service_response(response));
    }

    if optional_response_is_none::<E>(headers, payload)? {
        return Ok(None);
    }

    deserialize_response::<T, E>(headers, payload).map(Some)
}

/// Deserializes a protobuf response payload into the requested type.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when the payload cannot be decoded as the expected protobuf message.
pub fn deserialize_proto_response<
    T: crate::prost::Message + Default,
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<T, crate::ClientError<E>> {
    match response_success_from_headers::<E>(headers)? {
        Some(true) => {
            return T::decode(payload).map_err(|error| {
                crate::ClientError::deserialize(crate::NatsErrorResponse::framework(
                    FrameworkError::DeserializationError,
                    format!("failed to decode a successful response payload as protobuf: {error}"),
                ))
            });
        }
        Some(false) => {
            let response = crate::error::deserialize_error_response::<E>(payload)?;
            return Err(crate::ClientError::from_service_response(response));
        }
        None => {}
    }

    match T::decode(payload) {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                Err(crate::ClientError::from_service_response(response))
            } else {
                Err(crate::ClientError::deserialize(
                    crate::NatsErrorResponse::framework(
                        FrameworkError::DeserializationError,
                        format!("failed to decode the response payload as protobuf: {error}"),
                    ),
                ))
            }
        }
    }
}

/// Deserializes an optional protobuf response payload into the requested type.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when a present payload cannot be decoded as the expected protobuf message.
pub fn deserialize_optional_proto_response<
    T: crate::prost::Message + Default,
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Option<T>, crate::ClientError<E>> {
    if let Some(false) = response_success_from_headers::<E>(headers)? {
        let response = crate::error::deserialize_error_response::<E>(payload)?;
        return Err(crate::ClientError::from_service_response(response));
    }

    if optional_response_is_none::<E>(headers, payload)? {
        return Ok(None);
    }

    deserialize_proto_response::<T, E>(headers, payload).map(Some)
}

/// Validates that a unit response is empty and successful.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when a successful response includes an unexpected payload.
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
            return Err(crate::ClientError::invalid_response(
                NatsErrorResponse::framework(
                    FrameworkError::InvalidResponse,
                    format!(
                        "expected an empty response payload when `{}`=true, but received {} byte(s)",
                        crate::X_SUCCESS_HEADER,
                        payload.len()
                    ),
                ),
            ));
        }
        Some(false) => {
            let response = crate::error::deserialize_error_response::<E>(payload)?;
            return Err(crate::ClientError::from_service_response(response));
        }
        None => {}
    }

    if payload.is_empty() {
        return Ok(());
    }
    if let Some(response) = crate::error::try_deserialize_error_response(payload) {
        return Err(crate::ClientError::from_service_response(response));
    }
    Err(crate::ClientError::invalid_response(
        crate::NatsErrorResponse::framework(
            FrameworkError::InvalidResponse,
            format!(
                "expected an empty response payload, but received {} byte(s)",
                payload.len()
            ),
        ),
    ))
}

/// Validates that an optional unit response is empty when present.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when a present successful response includes an unexpected payload.
pub fn deserialize_optional_unit_response<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Option<()>, crate::ClientError<E>> {
    if let Some(false) = response_success_from_headers::<E>(headers)? {
        let response = crate::error::deserialize_error_response::<E>(payload)?;
        return Err(crate::ClientError::from_service_response(response));
    }

    if optional_response_is_none::<E>(headers, payload)? {
        return Ok(None);
    }

    deserialize_unit_response::<E>(headers, payload).map(Some)
}

/// Converts a successful raw response body into UTF-8 text.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when the payload is not valid UTF-8.
pub fn raw_response_to_string<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<String, crate::ClientError<E>> {
    match response_success_from_headers::<E>(headers)? {
        Some(true) => {}
        Some(false) => {
            let response = crate::error::deserialize_error_response::<E>(payload)?;
            return Err(crate::ClientError::from_service_response(response));
        }
        None => {
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                return Err(crate::ClientError::from_service_response(response));
            }
        }
    }

    String::from_utf8(payload.to_vec()).map_err(|error| {
        crate::ClientError::deserialize(crate::NatsErrorResponse::framework(
            FrameworkError::DeserializationError,
            format!("failed to decode the response payload as UTF-8 text: {error}"),
        ))
    })
}

/// Converts an optional successful raw response body into UTF-8 text.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when a present payload is not valid UTF-8.
pub fn raw_response_to_optional_string<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Option<String>, crate::ClientError<E>> {
    if let Some(false) = response_success_from_headers::<E>(headers)? {
        let response = crate::error::deserialize_error_response::<E>(payload)?;
        return Err(crate::ClientError::from_service_response(response));
    }

    if optional_response_is_none::<E>(headers, payload)? {
        return Ok(None);
    }

    raw_response_to_string::<E>(headers, payload).map(Some)
}

/// Returns the raw bytes from a successful response.
///
/// # Errors
///
/// Returns a typed service error for remote failures.
pub fn raw_response_to_bytes<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Vec<u8>, crate::ClientError<E>> {
    match response_success_from_headers::<E>(headers)? {
        Some(true) => {}
        Some(false) => {
            let response = crate::error::deserialize_error_response::<E>(payload)?;
            return Err(crate::ClientError::from_service_response(response));
        }
        None => {
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                return Err(crate::ClientError::from_service_response(response));
            }
        }
    }
    Ok(payload.to_vec())
}

/// Returns the raw bytes from an optional successful response.
///
/// # Errors
///
/// Returns a typed service error for remote failures.
pub fn raw_response_to_optional_bytes<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    payload: &[u8],
) -> Result<Option<Vec<u8>>, crate::ClientError<E>> {
    if let Some(false) = response_success_from_headers::<E>(headers)? {
        let response = crate::error::deserialize_error_response::<E>(payload)?;
        return Err(crate::ClientError::from_service_response(response));
    }

    if optional_response_is_none::<E>(headers, payload)? {
        return Ok(None);
    }

    raw_response_to_bytes::<E>(headers, payload).map(Some)
}

#[cfg(feature = "encryption")]
/// Decrypts a successful encrypted client response.
///
/// # Errors
///
/// Returns a typed service error for remote failures, or a transport error
/// when the ciphertext cannot be decrypted.
pub fn decrypt_client_response<
    E: crate::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    headers: Option<&crate::async_nats::HeaderMap>,
    eph_ctx: &crate::EphemeralContext,
    payload: &[u8],
) -> Result<Vec<u8>, crate::ClientError<E>> {
    if matches!(optional_response_from_headers::<E>(headers)?, Some(false)) {
        return Ok(Vec::new());
    }

    match response_success_from_headers::<E>(headers)? {
        Some(false) => return Ok(payload.to_vec()),
        Some(true) => {
            return eph_ctx.decrypt_response(payload).map_err(|error| {
                crate::ClientError::decrypt(crate::NatsErrorResponse::framework(
                    FrameworkError::DecryptError,
                    format!("failed to decrypt the successful response payload: {error}"),
                ))
            });
        }
        None => {}
    }

    match eph_ctx.decrypt_response(payload) {
        Ok(plaintext) => Ok(plaintext),
        Err(error) => {
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                Err(crate::ClientError::from_service_response(response))
            } else {
                Err(crate::ClientError::decrypt(
                    crate::NatsErrorResponse::framework(
                        FrameworkError::DecryptError,
                        format!("failed to decrypt the response payload: {error}"),
                    ),
                ))
            }
        }
    }
}

/// Serializes a serde payload into bytes for client transport.
///
/// # Errors
///
/// Returns a framework error when the payload cannot be serialized to JSON.
pub fn serialize_serde_payload<T: crate::serde::Serialize>(
    payload: &T,
) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
    crate::serde_json::to_vec(payload)
        .map(::bytes::Bytes::from)
        .map_err(|e| {
            crate::NatsErrorResponse::framework(
                FrameworkError::SerializationError,
                format!("failed to serialize the request payload as JSON: {e}"),
            )
        })
}

/// Serializes a protobuf payload into bytes for client transport.
///
/// # Errors
///
/// Returns a framework error when the payload cannot be encoded as protobuf.
pub fn serialize_proto_payload<T: crate::prost::Message>(
    payload: &T,
) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
    let mut buf = Vec::new();
    payload.encode(&mut buf).map_err(|e| {
        crate::NatsErrorResponse::framework(
            FrameworkError::SerializationError,
            format!("failed to encode the request payload as protobuf: {e}"),
        )
    })?;
    Ok(::bytes::Bytes::from(buf))
}
