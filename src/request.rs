use async_nats::HeaderMap;
use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct NatsRequest {
    pub subject: String,
    pub payload: Bytes,
    pub headers: HeaderMap,
    pub reply: Option<String>,
    pub request_id: String,
}

impl NatsRequest {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}
