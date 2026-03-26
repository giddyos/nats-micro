use bytes::Bytes;
use serde::Serialize;

use crate::{error::NatsErrorResponse, extractors::Json};

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

    pub fn json<T: Serialize>(value: &T) -> Result<Self, NatsErrorResponse> {
        let payload = serde_json::to_vec(value)
            .map_err(|e| NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string()))?;
        Ok(Self::new(payload))
    }
}

pub trait IntoNatsResponse {
    fn into_response(self, request_id: String) -> Result<NatsResponse, NatsErrorResponse>;
}

impl IntoNatsResponse for NatsResponse {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(self)
    }
}

impl IntoNatsResponse for () {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(Bytes::new()))
    }
}

impl IntoNatsResponse for Bytes {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for Vec<u8> {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for String {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl IntoNatsResponse for &'static str {
    fn into_response(self, _request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        Ok(NatsResponse::new(self))
    }
}

impl<T> IntoNatsResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self, request_id: String) -> Result<NatsResponse, NatsErrorResponse> {
        NatsResponse::json(&self.0)
            .map_err(|e| e.with_request_id(request_id))
    }
}
