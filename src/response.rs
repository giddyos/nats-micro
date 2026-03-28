use bytes::Bytes;
use serde::Serialize;

use crate::{Proto, error::NatsErrorResponse, extractors::Json, handler::RequestContext};

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
        serde_json::to_vec(&self.0)
            .map_err(|e| {
                NatsErrorResponse::internal("SERIALIZATION_ERROR", e.to_string())
                    .with_request_id(ctx.request.request_id.clone())
            })
            .map(NatsResponse::new)
    }
}

impl<T> IntoNatsResponse for Proto<T>
where
    T: prost::Message,
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
