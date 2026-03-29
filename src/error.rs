use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsErrorResponse {
    pub code: u16,
    pub error: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl NatsErrorResponse {
    pub fn new(
        code: u16,
        error: impl Into<String>,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            code,
            error: error.into(),
            message: message.into(),
            request_id: request_id.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }

    pub fn bad_request(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(400, error, message, "")
    }

    pub fn unauthorized(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(401, error, message, "")
    }

    pub fn forbidden(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(403, error, message, "")
    }

    pub fn not_found(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(404, error, message, "")
    }

    pub fn internal(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(500, error, message, "")
    }
}

impl std::fmt::Display for NatsErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.error, self.message)
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
    #[error("{0}")]
    Service(E),
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
    pub fn from_service_response(response: NatsErrorResponse) -> Self {
        match E::from_nats_error_response(response) {
            ServiceErrorMatch::Typed(error) => Self::Service(error),
            ServiceErrorMatch::Untyped(response) => Self::ServiceResponse(response),
        }
    }

    pub fn request(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Request(response))
    }

    pub fn serialize(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Serialize(response))
    }

    pub fn deserialize(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Deserialize(response))
    }

    pub fn decrypt(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::Decrypt(response))
    }

    pub fn invalid_response(response: NatsErrorResponse) -> Self {
        Self::Transport(ClientTransportError::InvalidResponse(response))
    }

    pub fn as_nats_error_response(&self) -> Option<&NatsErrorResponse> {
        match self {
            Self::Service(_) => None,
            Self::ServiceResponse(response) => Some(response),
            Self::Transport(error) => Some(error.as_nats_error_response()),
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

impl IntoNatsError for anyhow::Error {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        // Preserve a safe, truncated error message in `details` to aid debugging
        // while avoiding leaking large or sensitive payloads.
        let msg = self.to_string();
        let trunc = if msg.len() > 200 { &msg[..200] } else { &msg };
        NatsErrorResponse::internal("INTERNAL_ERROR", "an internal error occurred")
            .with_details(Value::String(trunc.to_string()))
            .with_request_id(request_id)
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("startup error: {0}")]
    Startup(String),

    #[error("transport error: {0}")]
    Transport(String),
}

impl IntoNatsError for AppError {
    fn into_nats_error(self, request_id: String) -> NatsErrorResponse {
        match self {
            AppError::Startup(msg) => {
                NatsErrorResponse::internal("STARTUP_ERROR", msg).with_request_id(request_id)
            }
            AppError::Transport(msg) => {
                NatsErrorResponse::internal("TRANSPORT_ERROR", msg).with_request_id(request_id)
            }
        }
    }
}
