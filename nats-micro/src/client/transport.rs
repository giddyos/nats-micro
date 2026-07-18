use std::{future::Future, time::Duration};

use async_nats::HeaderMap;
use bytes::Bytes;

#[derive(Debug, Clone)]
pub enum Subject<'a> {
    Static(&'static str),
    Borrowed(&'a str),
    Owned(String),
}

impl Subject<'_> {
    #[inline]
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(value) => value,
            Self::Borrowed(value) => value,
            Self::Owned(value) => value.as_str(),
        }
    }

    fn into_nats_subject(self) -> async_nats::Subject {
        match self {
            Self::Static(value) => async_nats::Subject::from_static(value),
            Self::Borrowed(value) => async_nats::Subject::from(value),
            Self::Owned(value) => async_nats::Subject::from(value),
        }
    }
}

#[derive(Debug)]
pub struct ClientRequest<'a> {
    pub subject: Subject<'a>,
    pub payload: Bytes,
    pub headers: Option<HeaderMap>,
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct ClientResponse {
    pub payload: Bytes,
    pub headers: Option<HeaderMap>,
}

pub trait ClientTransport: Clone + Send + Sync + 'static {
    fn request<'a>(
        &'a self,
        request: ClientRequest<'a>,
    ) -> impl Future<Output = Result<ClientResponse, crate::TransportError>> + Send + 'a;

    fn publish<'a>(
        &'a self,
        subject: Subject<'a>,
        payload: Bytes,
        headers: Option<HeaderMap>,
    ) -> impl Future<Output = Result<(), crate::TransportError>> + Send + 'a;
}

#[derive(Debug, Clone)]
pub struct NatsTransport {
    client: async_nats::Client,
}

impl NatsTransport {
    #[must_use]
    pub const fn new(client: async_nats::Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub const fn client(&self) -> &async_nats::Client {
        &self.client
    }
}

impl From<async_nats::Client> for NatsTransport {
    fn from(client: async_nats::Client) -> Self {
        Self::new(client)
    }
}

impl ClientTransport for NatsTransport {
    async fn request<'a>(
        &'a self,
        request: ClientRequest<'a>,
    ) -> Result<ClientResponse, crate::TransportError> {
        let mut nats_request = async_nats::Request::new().payload(request.payload);
        if let Some(headers) = request.headers {
            nats_request = nats_request.headers(headers);
        }
        if let Some(timeout) = request.timeout {
            nats_request = nats_request.timeout(Some(timeout));
        }
        let subject = request.subject.into_nats_subject();
        let response = self
            .client
            .send_request(subject, nats_request)
            .await
            .map_err(|error| match error.kind() {
                async_nats::RequestErrorKind::TimedOut => crate::TransportError::Timeout,
                async_nats::RequestErrorKind::NoResponders => crate::TransportError::NoResponders,
                async_nats::RequestErrorKind::InvalidSubject
                | async_nats::RequestErrorKind::MaxPayloadExceeded
                | async_nats::RequestErrorKind::Other => crate::TransportError::NatsRequestFailed,
            })?;
        Ok(ClientResponse {
            payload: response.payload,
            headers: response.headers,
        })
    }

    async fn publish<'a>(
        &'a self,
        subject: Subject<'a>,
        payload: Bytes,
        headers: Option<HeaderMap>,
    ) -> Result<(), crate::TransportError> {
        let subject = subject.into_nats_subject();
        if let Some(headers) = headers {
            self.client
                .publish_with_headers(subject, headers, payload)
                .await
                .map_err(|error| match error.kind() {
                    async_nats::PublishErrorKind::Send => crate::TransportError::Disconnected,
                    async_nats::PublishErrorKind::InvalidSubject
                    | async_nats::PublishErrorKind::MaxPayloadExceeded => {
                        crate::TransportError::TransportError
                    }
                })?;
        } else {
            self.client
                .publish(subject, payload)
                .await
                .map_err(|error| match error.kind() {
                    async_nats::PublishErrorKind::Send => crate::TransportError::Disconnected,
                    async_nats::PublishErrorKind::InvalidSubject
                    | async_nats::PublishErrorKind::MaxPayloadExceeded => {
                        crate::TransportError::TransportError
                    }
                })?;
        }
        Ok(())
    }
}
