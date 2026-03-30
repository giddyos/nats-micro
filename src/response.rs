use bytes::Bytes;

use crate::{
    Proto, error::NatsErrorResponse, extractors::Json, handler::RequestContext,
    serde::Serialize,
};

pub const X_SUCCESS_HEADER: &str = "x-success";

#[derive(Debug, Clone)]
pub struct NatsResponse {
    pub payload: Bytes,
}

impl NatsResponse {
    pub fn new(payload: impl Into<Bytes>) -> Self {
        Self {
            payload: payload.into(),
        }
    }
}

pub trait IntoNatsResponse {
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

impl<T> IntoNatsResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self, ctx: &RequestContext) -> Result<NatsResponse, NatsErrorResponse> {
        crate::serde_json::to_vec(&self.0)
            .map_err(|e| {
                NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string())
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
            NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string())
                .with_request_id(ctx.request.request_id.clone())
        })?;
        Ok(NatsResponse::new(buf))
    }
}

// Client / response decoding helpers used by generated clients and macros.
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
            NatsErrorResponse::internal(
                "INVALID_RESPONSE",
                format!("invalid {} header value: {value}", X_SUCCESS_HEADER),
            ),
        ))
    }
}

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
                crate::ClientError::deserialize(crate::NatsErrorResponse::internal(
                    "DESERIALIZATION_ERROR",
                    error.to_string(),
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
                    crate::NatsErrorResponse::internal("DESERIALIZATION_ERROR", error.to_string()),
                ))
            }
        }
    }
}

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
                crate::ClientError::deserialize(crate::NatsErrorResponse::internal(
                    "DESERIALIZATION_ERROR",
                    error.to_string(),
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
                    crate::NatsErrorResponse::internal("DESERIALIZATION_ERROR", error.to_string()),
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
            return Err(crate::ClientError::invalid_response(
                NatsErrorResponse::internal(
                    "INVALID_RESPONSE",
                    "expected empty response payload when x-success=true",
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
            if let Some(response) = crate::error::try_deserialize_error_response(payload) {
                Err(crate::ClientError::from_service_response(response))
            } else {
                Err(crate::ClientError::decrypt(
                    crate::NatsErrorResponse::internal("DECRYPT_ERROR", error.to_string()),
                ))
            }
        }
    }
}

pub fn serialize_serde_payload<T: crate::serde::Serialize>(
    payload: &T,
) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
    crate::serde_json::to_vec(payload)
        .map(::bytes::Bytes::from)
        .map_err(|e| crate::NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string()))
}

pub fn serialize_proto_payload<T: crate::prost::Message>(
    payload: &T,
) -> Result<::bytes::Bytes, crate::NatsErrorResponse> {
    let mut buf = Vec::new();
    payload
        .encode(&mut buf)
        .map_err(|e| crate::NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string()))?;
    Ok(::bytes::Bytes::from(buf))
}
