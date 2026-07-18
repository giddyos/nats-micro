use std::borrow::Cow;

use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro_shared::FrameworkError;
use serde::Serialize;

pub const PRESENT_HEADER: &str = "Nats-Micro-Present";

/// A static endpoint response.
#[derive(Debug)]
pub enum Response {
    Empty,
    Payload(Bytes),
    WithHeaders { payload: Bytes, headers: HeaderMap },
}

impl Response {
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self::Empty
    }

    #[inline]
    #[must_use]
    pub fn bytes(bytes: impl Into<Bytes>) -> Self {
        Self::Payload(bytes.into())
    }

    #[inline]
    #[must_use]
    pub fn with_headers(payload: impl Into<Bytes>, headers: HeaderMap) -> Self {
        Self::WithHeaders {
            payload: payload.into(),
            headers,
        }
    }

    /// Encodes `None` without making an empty payload ambiguous with a present
    /// value. `Some` values use their ordinary response encoding.
    #[inline]
    #[must_use]
    pub fn optional_none() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(PRESENT_HEADER, "0");
        Self::WithHeaders {
            payload: Bytes::new(),
            headers,
        }
    }
}

/// A protocol error ready to send through native NATS service error headers.
#[derive(Debug)]
pub struct ErrorReply {
    pub code: u16,
    pub kind: Cow<'static, str>,
    pub message: String,
    pub payload: Bytes,
    pub request_id: Option<String>,
}

#[derive(Serialize)]
struct WireError<'a> {
    code: u16,
    kind: &'static str,
    message: &'a str,
    request_id: &'a str,
}

impl ErrorReply {
    #[must_use]
    pub fn new(
        code: u16,
        kind: &'static str,
        message: impl Into<String>,
        request_id: Option<&str>,
    ) -> Self {
        let message = message.into();
        let request_id = request_id.map(str::to_owned);
        let wire = WireError {
            code,
            kind,
            message: &message,
            request_id: request_id.as_deref().unwrap_or_default(),
        };
        let payload = serde_json::to_vec(&wire).map_or_else(|_| Bytes::new(), Bytes::from);

        Self {
            code,
            kind: Cow::Borrowed(kind),
            message,
            payload,
            request_id,
        }
    }

    #[must_use]
    pub fn framework(
        error: FrameworkError,
        message: impl Into<String>,
        request_id: Option<&str>,
    ) -> Self {
        Self::new(error.status_code(), error.as_code(), message, request_id)
    }

    #[must_use]
    pub fn with_payload(mut self, payload: impl Into<Bytes>) -> Self {
        self.payload = payload.into();
        self
    }

    #[must_use]
    pub fn from_nats_error(error: crate::NatsErrorResponse) -> Self {
        let payload = serde_json::to_vec(&error).map_or_else(|_| Bytes::new(), Bytes::from);
        let request_id = (!error.request_id.is_empty()).then(|| error.request_id.clone());
        Self {
            code: error.code,
            kind: Cow::Owned(error.kind),
            message: error.message,
            payload,
            request_id,
        }
    }

    #[must_use]
    pub fn missing_subject_parameter(name: &str, request_id: Option<&str>) -> Self {
        Self::framework(
            FrameworkError::SubjectParamMissing,
            format!("subject parameter `{name}` is missing"),
            request_id,
        )
    }

    #[must_use]
    pub fn invalid_subject_parameter(
        name: &str,
        error: impl std::fmt::Display,
        request_id: Option<&str>,
    ) -> Self {
        Self::framework(
            FrameworkError::SubjectParamInvalid,
            format!("subject parameter `{name}` is invalid: {error}"),
            request_id,
        )
    }

    #[must_use]
    pub fn missing_header(name: &str, request_id: Option<&str>) -> Self {
        Self::framework(
            FrameworkError::InvalidHeader,
            format!("required header `{name}` is missing"),
            request_id,
        )
    }
}

pub(crate) fn service_error_headers(error: &ErrorReply) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Service-Error", error.message.as_str());
    headers.insert("Nats-Service-Error-Code", error.code.to_string());
    headers.insert("Nats-Micro-Error-Kind", error.kind.as_ref());
    headers.insert("Nats-Micro-Error-Format", "json-v1");
    if let Some(request_id) = error.request_id.as_deref() {
        headers.insert("x-request-id", request_id);
    }
    headers
}

/// Converts a typed service failure into the static response protocol.
pub trait IntoServiceError {
    fn into_service_error(self, request_id: &crate::RequestId<'_>) -> ErrorReply;
}

impl IntoServiceError for ErrorReply {
    fn into_service_error(self, _: &crate::RequestId<'_>) -> ErrorReply {
        self
    }
}

impl<T> IntoServiceError for T
where
    T: crate::IntoNatsError,
{
    fn into_service_error(self, request_id: &crate::RequestId<'_>) -> ErrorReply {
        let error = self.into_nats_error(request_id.get_or_generate().to_owned());
        ErrorReply::from_nats_error(error)
    }
}
