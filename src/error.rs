use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use nats_micro_shared::{FrameworkError, TransportError as SharedTransportError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsErrorResponse {
    pub code: u16,
    pub kind: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl NatsErrorResponse {
    #[must_use]
    pub fn new(
        code: u16,
        kind: impl Into<String>,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            code,
            kind: kind.into(),
            message: message.into(),
            request_id: request_id.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }

    #[must_use]
    pub fn bad_request(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(400, error, message, "")
    }

    #[must_use]
    pub fn unauthorized(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(401, error, message, "")
    }

    #[must_use]
    pub fn forbidden(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(403, error, message, "")
    }

    #[must_use]
    pub fn not_found(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(404, error, message, "")
    }

    #[must_use]
    pub fn internal(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(500, error, message, "")
    }

    #[must_use]
    pub fn framework(error: FrameworkError, message: impl Into<String>) -> Self {
        Self::new(error.status_code(), error.as_code(), message, "")
    }

    #[must_use]
    pub fn transport(error: SharedTransportError, message: impl Into<String>) -> Self {
        Self::new(error.status_code(), error.as_code(), message, "")
    }
}

impl std::fmt::Display for NatsErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.kind, self.message)
    }
}

impl std::error::Error for NatsErrorResponse {}

pub type NatsError = NatsErrorResponse;

/// Distinguishes between service errors that can be losslessly reconstructed
/// into the caller's domain type and remote error payloads that should remain
/// as opaque protocol errors.
///
/// This split keeps the client error space closed without pretending every
/// remote failure can be projected back into a local enum variant.
#[derive(Debug, Clone)]
pub enum ServiceErrorMatch<E> {
    Typed(E),
    Untyped(NatsErrorResponse),
}

/// Rebuilds a typed service error from the wire representation when that is a
/// sound conversion, otherwise preserves the original protocol error.
///
/// The trait consumes the response rather than borrowing it so implementations
/// can move owned detail payloads into the typed error and avoid partial
/// reconstruction APIs like `Option<T>`.
pub trait FromNatsErrorResponse: Sized {
    fn from_nats_error_response(response: NatsErrorResponse) -> ServiceErrorMatch<Self>;
}

impl FromNatsErrorResponse for NatsErrorResponse {
    fn from_nats_error_response(response: NatsErrorResponse) -> ServiceErrorMatch<Self> {
        ServiceErrorMatch::Typed(response)
    }
}

impl FromNatsErrorResponse for anyhow::Error {
    fn from_nats_error_response(response: NatsErrorResponse) -> ServiceErrorMatch<Self> {
        ServiceErrorMatch::Typed(anyhow::anyhow!(response.to_string()))
    }
}

impl FromNatsErrorResponse for () {
    fn from_nats_error_response(response: NatsErrorResponse) -> ServiceErrorMatch<Self> {
        ServiceErrorMatch::Untyped(response)
    }
}

/// Captures the client-side failure mode separately from the remote service's
/// business error space.
///
/// The variants are intentionally narrow: a failure came from request
/// transport, request serialization, response decoding, response decryption, or
/// an unexpected response shape. Grouping by phase makes matching more useful
/// than returning another undifferentiated `NatsErrorResponse`.
#[derive(Debug, Error)]
pub enum ClientTransportError {
    #[error("request transport failed: {0}")]
    Request(NatsErrorResponse),
    #[error("request serialization failed: {0}")]
    Serialize(NatsErrorResponse),
    #[error("response deserialization failed: {0}")]
    Deserialize(NatsErrorResponse),
    #[error("response decryption failed: {0}")]
    Decrypt(NatsErrorResponse),
    #[error("invalid response shape: {0}")]
    InvalidResponse(NatsErrorResponse),
}

impl ClientTransportError {
    #[must_use]
    pub fn as_nats_error_response(&self) -> &NatsErrorResponse {
        match self {
            Self::Request(response)
            | Self::Serialize(response)
            | Self::Deserialize(response)
            | Self::Decrypt(response)
            | Self::InvalidResponse(response) => response,
        }
    }
}

#[cfg(feature = "napi")]
impl NatsErrorResponse {
    #[must_use]
    pub fn into_rust_napi_error(self) -> crate::napi::Error<String> {
        crate::napi::Error::new(self.kind, self.message)
    }

    fn into_napi_error_with_flag(
        self,
        env: crate::napi::Env,
        flag_field: Option<&str>,
    ) -> crate::napi::Error {
        let Self {
            code,
            kind,
            message,
            request_id,
            details,
        } = self;
        let fallback_message = message.clone();

        (|| -> crate::napi::Result<crate::napi::Error> {
            let mut error = env.create_error(crate::napi::Error::from_reason(message))?;
            if let Some(flag_field) = flag_field {
                error.set(flag_field, true)?;
            }
            error.set("code", kind.clone())?;
            error.set("name", kind.clone())?;
            error.set("kind", kind.clone())?;
            error.set("statusCode", code)?;
            error.set("requestId", request_id)?;
            if let Some(details) = details {
                error.set("details", details.to_string())?;
            }

            let unknown = crate::napi::bindgen_prelude::ToNapiValue::into_unknown(error, &env)?;
            Ok(crate::napi::Error::from(unknown))
        })()
        .unwrap_or_else(|_| crate::napi::Error::from_reason(fallback_message))
    }

    #[must_use]
    pub fn into_napi_error(self, env: crate::napi::Env) -> crate::napi::Error {
        self.into_napi_error_with_flag(env, Some("isServiceError"))
    }
}

#[cfg(feature = "napi")]
fn is_transport_error_kind(kind: &str) -> bool {
    SharedTransportError::from_code(kind).is_some()
}

#[cfg(feature = "napi")]
fn is_framework_error_kind(kind: &str) -> bool {
    FrameworkError::from_code(kind).is_some()
}

#[cfg(feature = "napi")]
#[derive(Debug, Clone)]
pub enum NapiClientError {
    Service(NatsErrorResponse),
    Framework(NatsErrorResponse),
    Transport(NatsErrorResponse),
    Generic(NatsErrorResponse),
}

#[cfg(feature = "napi")]
impl NapiClientError {
    #[must_use]
    pub fn service(response: NatsErrorResponse) -> Self {
        Self::Service(response)
    }

    #[must_use]
    pub fn framework(response: NatsErrorResponse) -> Self {
        Self::Framework(response)
    }

    #[must_use]
    pub fn transport(response: NatsErrorResponse) -> Self {
        Self::Transport(response)
    }

    #[must_use]
    pub fn generic(response: NatsErrorResponse) -> Self {
        Self::Generic(response)
    }

    #[must_use]
    pub fn from_response(response: NatsErrorResponse) -> Self {
        if is_transport_error_kind(response.kind.as_str()) {
            Self::Transport(response)
        } else if is_framework_error_kind(response.kind.as_str()) {
            Self::Framework(response)
        } else {
            Self::Generic(response)
        }
    }

    #[must_use]
    pub fn into_rust_napi_error(self) -> crate::napi::Error<String> {
        match self {
            Self::Service(response)
            | Self::Framework(response)
            | Self::Transport(response)
            | Self::Generic(response) => response.into_rust_napi_error(),
        }
    }

    #[must_use]
    pub fn into_napi_error(self, env: crate::napi::Env) -> crate::napi::Error {
        match self {
            Self::Service(response) => response.into_napi_error(env),
            Self::Framework(response) => {
                response.into_napi_error_with_flag(env, Some("isFrameworkError"))
            }
            Self::Transport(response) => {
                response.into_napi_error_with_flag(env, Some("isTransportError"))
            }
            Self::Generic(response) => response.into_napi_error_with_flag(env, None),
        }
    }
}

/// A generated client call can fail in only three ways:
///
/// - the remote service returned a typed domain error
/// - the remote service returned an error that cannot be safely projected into
///   the caller's domain enum
/// - the client failed before it could produce a valid domain result
///
/// Keeping these cases explicit makes exhaustive matching possible and avoids
/// conflating protocol failures with service-level business errors.
#[derive(Debug, Error)]
pub enum ClientError<E>
where
    E: std::fmt::Debug + std::fmt::Display + 'static,
{
    #[error("{error}")]
    Service {
        error: E,
        response: NatsErrorResponse,
    },
    #[error("{0}")]
    ServiceResponse(NatsErrorResponse),
    #[error("{0}")]
    Transport(ClientTransportError),
}

impl<E> ClientError<E>
where
    E: FromNatsErrorResponse + std::fmt::Debug + std::fmt::Display + 'static,
{
    /// Converts a remote error payload into either a typed service error or an
    /// untyped protocol error. The conversion is total so callers never need a
    /// second fallback channel.
    #[must_use]
    pub fn from_service_response(response: NatsErrorResponse) -> Self {
        match E::from_nats_error_response(response.clone()) {
            ServiceErrorMatch::Typed(error) => Self::Service { error, response },
            ServiceErrorMatch::Untyped(response) => Self::ServiceResponse(response),
        }
    }

    #[must_use]
    pub fn request(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Request(response))
    }

    #[must_use]
    pub fn serialize(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Serialize(response))
    }

    #[must_use]
    pub fn deserialize(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Deserialize(response))
    }

    #[must_use]
    pub fn decrypt(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Decrypt(response))
    }

    #[must_use]
    pub fn invalid_response(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::InvalidResponse(response))
    }

    #[must_use]
    pub fn as_nats_error_response(&self) -> Option<&NatsErrorResponse> {
        match self {
            Self::Service { response, .. } | Self::ServiceResponse(response) => Some(response),
            Self::Transport(error) => Some(error.as_nats_error_response()),
        }
    }
}

impl<E> ClientError<E>
where
    E: std::fmt::Debug + std::fmt::Display + 'static,
{
    #[must_use]
    pub fn into_nats_error_response(self) -> NatsErrorResponse {
        match self {
            Self::Service { response, .. } | Self::ServiceResponse(response) => response,
            Self::Transport(error) => error.as_nats_error_response().clone(),
        }
    }
}

pub trait IntoNatsError {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse;
}

impl IntoNatsError for NatsErrorResponse {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        self.with_request_id(request_id)
    }
}

impl IntoNatsError for String {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        NatsErrorResponse::framework(FrameworkError::Error, self).with_request_id(request_id)
    }
}

impl IntoNatsError for anyhow::Error {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        // Preserve a safe, truncated error message in `details` to aid debugging
        // while avoiding leaking large or sensitive payloads.
        let msg = self.to_string();
        let trunc = if msg.len() > 200 { &msg[..200] } else { &msg };
        NatsErrorResponse::framework(FrameworkError::InternalError, "an internal error occurred")
            .with_details(Value::String(trunc.to_string()))
            .with_request_id(request_id)
    }
}

// Helper utilities for deserializing error responses coming from services.
#[must_use]
pub fn deserialize_error_response_payload(payload: &[u8]) -> Option<NatsErrorResponse> {
    serde_json::from_slice::<NatsErrorResponse>(payload).ok()
}

#[must_use]
pub fn invalid_response(message: impl Into<String>) -> NatsErrorResponse {
    NatsErrorResponse::framework(FrameworkError::InvalidResponse, message.into())
}

/// Deserializes a protocol error payload when a client has already determined
/// the remote service reported failure.
///
/// # Errors
///
/// Returns a client transport error when the payload is not a serialized
/// [`NatsErrorResponse`].
pub fn deserialize_error_response<
    E: FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static,
>(
    payload: &[u8],
) -> Result<NatsErrorResponse, ClientError<E>> {
    deserialize_error_response_payload(payload).ok_or_else(|| {
        ClientError::invalid_response(invalid_response(format!(
            "expected serialized NatsErrorResponse payload when {}=false",
            crate::X_SUCCESS_HEADER,
        )))
    })
}

#[must_use]
pub fn try_deserialize_error_response(payload: &[u8]) -> Option<NatsErrorResponse> {
    deserialize_error_response_payload(payload).filter(|error| error.code >= 400)
}

#[cfg(all(test, feature = "napi"))]
mod tests {
    use super::{NapiClientError, NatsErrorResponse};
    use nats_micro_shared::{FrameworkError, TransportError as SharedTransportError};

    #[test]
    fn napi_client_error_classifies_framework_responses() {
        let response = NatsErrorResponse::framework(
            FrameworkError::MissingRecipientPubkey,
            "recipient required",
        );
        assert!(matches!(
            NapiClientError::from_response(response),
            NapiClientError::Framework(_)
        ));
    }

    #[test]
    fn napi_client_error_classifies_transport_responses() {
        let response =
            NatsErrorResponse::transport(SharedTransportError::NatsRequestFailed, "request failed");
        assert!(matches!(
            NapiClientError::from_response(response),
            NapiClientError::Transport(_)
        ));
    }

    #[test]
    fn napi_client_error_leaves_unknown_responses_generic() {
        let response = NatsErrorResponse::internal("SOME_CUSTOM_KIND", "custom");
        assert!(matches!(
            NapiClientError::from_response(response),
            NapiClientError::Generic(_)
        ));
    }
}
