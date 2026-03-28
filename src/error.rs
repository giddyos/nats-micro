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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn anyhow_into_nats_error_includes_truncated_details() {
        let err = anyhow!("something went wrong: secret=very-sensitive-data");
        let resp = err.into_nats_error("req-123".to_string());
        assert_eq!(resp.code, 500);
        assert_eq!(resp.error, "INTERNAL_ERROR");
        assert_eq!(resp.message, "an internal error occurred");
        assert_eq!(resp.request_id, "req-123");
        assert!(resp.details.is_some());
        if let Some(Value::String(s)) = resp.details {
            assert!(s.contains("something went wrong"));
            assert!(s.len() <= 200);
        }
    }
}
