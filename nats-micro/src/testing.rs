use std::{future::Future, sync::Arc};

use async_nats::HeaderMap;
use bytes::Bytes;

pub use nats_micro_testing::{
    CapturedEvent, EventLog, EventSelection, FaultPlan, init_test_tracing,
};

use crate::{
    ClientRequest, ClientResponse, ClientTransport, Cons, Nil, RequestEndpoint, Response, Service,
    ServiceSet,
};

#[derive(Debug)]
pub enum LocalSubject {
    Static(&'static str),
    Owned(String),
}

impl LocalSubject {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(value) => value,
            Self::Owned(value) => value,
        }
    }
}

impl<'a> From<crate::ClientSubject<'a>> for LocalSubject {
    fn from(subject: crate::ClientSubject<'a>) -> Self {
        match subject {
            crate::ClientSubject::Static(value) => Self::Static(value),
            crate::ClientSubject::Borrowed(value) => Self::Owned(value.to_owned()),
            crate::ClientSubject::Owned(value) => Self::Owned(value),
        }
    }
}

#[derive(Debug)]
pub struct LocalRequest {
    subject: LocalSubject,
    payload: Bytes,
    headers: Option<HeaderMap>,
}

impl LocalRequest {
    #[must_use]
    pub fn subject(&self) -> &str {
        self.subject.as_str()
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&HeaderMap> {
        self.headers.as_ref()
    }

    #[must_use]
    pub fn view(&self) -> crate::Request<'_> {
        crate::Request::new(
            self.subject(),
            None,
            self.payload.as_ref(),
            self.headers.as_ref(),
        )
    }
}

#[derive(Debug)]
pub enum LocalDispatch {
    Matched(Result<ClientResponse, crate::TransportError>),
    NotMatched(LocalRequest),
}

pub trait LocalServiceSet<S>: Send + Sync + 'static {
    fn dispatch<'a>(
        &'a self,
        state: &'a S,
        request: LocalRequest,
    ) -> impl Future<Output = LocalDispatch> + Send + 'a;
}

impl<S> LocalServiceSet<S> for Nil
where
    S: Send + Sync + 'static,
{
    async fn dispatch<'a>(&'a self, _state: &'a S, request: LocalRequest) -> LocalDispatch {
        LocalDispatch::NotMatched(request)
    }
}

impl<S, Head, Tail> LocalServiceSet<S> for Cons<Head, Tail>
where
    S: Send + Sync + 'static,
    Head: Service<S>,
    Tail: LocalServiceSet<S>,
{
    async fn dispatch<'a>(&'a self, state: &'a S, request: LocalRequest) -> LocalDispatch {
        match self.head.dispatch_local(state, request).await {
            LocalDispatch::Matched(response) => LocalDispatch::Matched(response),
            LocalDispatch::NotMatched(request) => self.tail.dispatch(state, request).await,
        }
    }
}

struct Inner<S, Services> {
    state: S,
    services: Services,
    events: EventLog,
    faults: FaultPlan,
}

pub struct LocalTransport<S, Services> {
    inner: Arc<Inner<S, Services>>,
}

impl<S, Services> Clone for LocalTransport<S, Services> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S, Services> ClientTransport for LocalTransport<S, Services>
where
    S: Send + Sync + 'static,
    Services: LocalServiceSet<S>,
{
    async fn request<'a>(
        &'a self,
        request: ClientRequest<'a>,
    ) -> Result<ClientResponse, crate::TransportError> {
        self.inner.faults.before_request(request.subject.as_str())?;

        let subject = request.subject.into();
        let timeout = request.timeout;
        let request = LocalRequest {
            subject,
            payload: request.payload,
            headers: request.headers,
        };
        let dispatch = self.inner.services.dispatch(&self.inner.state, request);
        let dispatch = if let Some(timeout) = timeout {
            tokio::time::timeout(timeout, dispatch)
                .await
                .map_err(|_| crate::TransportError::Timeout)?
        } else {
            dispatch.await
        };

        match dispatch {
            LocalDispatch::Matched(response) => response,
            LocalDispatch::NotMatched(_) => Err(crate::TransportError::NoResponders),
        }
    }

    async fn publish<'a>(
        &'a self,
        subject: crate::ClientSubject<'a>,
        payload: Bytes,
        headers: Option<HeaderMap>,
    ) -> Result<(), crate::TransportError> {
        self.inner.faults.before_publish(subject.as_str())?;
        self.inner.events.record(subject.as_str(), payload, headers);
        Ok(())
    }
}

pub struct TestApp<S, Services = Nil> {
    state: S,
    services: Services,
    events: EventLog,
    faults: FaultPlan,
}

impl<S> TestApp<S, Nil>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(state: S) -> Self {
        Self {
            state,
            services: Nil,
            events: EventLog::default(),
            faults: FaultPlan::default(),
        }
    }
}

impl TestApp<(), Nil> {
    #[must_use]
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S, Services> TestApp<S, Services>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn serve<New>(self, service: New) -> TestApp<S, Cons<New, Services>>
    where
        New: Service<S>,
    {
        TestApp {
            state: self.state,
            services: Cons {
                head: service,
                tail: self.services,
            },
            events: self.events,
            faults: self.faults,
        }
    }

    #[must_use]
    pub fn start(self) -> TestHarness<S, Services>
    where
        Services: LocalServiceSet<S> + ServiceSet<S>,
    {
        crate::validate_service_set::<S, Services>(&self.services)
            .expect("test application service validation failed");
        let inner = Arc::new(Inner {
            state: self.state,
            services: self.services,
            events: self.events,
            faults: self.faults,
        });
        TestHarness {
            transport: LocalTransport { inner },
        }
    }
}

pub struct TestHarness<S, Services> {
    transport: LocalTransport<S, Services>,
}

impl<S, Services> TestHarness<S, Services>
where
    S: Send + Sync + 'static,
    Services: LocalServiceSet<S>,
{
    #[must_use]
    pub fn client<ServiceType>(&self) -> ServiceType::Client<LocalTransport<S, Services>>
    where
        ServiceType: Service<S>,
    {
        ServiceType::client(self.transport.clone())
    }

    #[must_use]
    pub fn transport(&self) -> LocalTransport<S, Services> {
        self.transport.clone()
    }

    #[must_use]
    pub fn events(&self) -> &EventLog {
        &self.transport.inner.events
    }

    #[must_use]
    pub fn faults(&self) -> &FaultPlan {
        &self.transport.inner.faults
    }
}

pub async fn dispatch<S, E>(state: &S, request: LocalRequest) -> LocalDispatch
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    let response = match E::call(state, request.view()).await {
        Ok(Response::Empty) => ClientResponse {
            payload: Bytes::new(),
            headers: None,
        },
        Ok(Response::Payload(payload)) => ClientResponse {
            payload,
            headers: None,
        },
        Ok(Response::WithHeaders { payload, headers }) => ClientResponse {
            payload,
            headers: Some(headers),
        },
        Err(error) => ClientResponse {
            headers: Some(crate::response::service_error_headers(&error)),
            payload: error.payload,
        },
    };
    LocalDispatch::Matched(Ok(response))
}

#[cfg(test)]
mod tests {
    use super::LocalSubject;
    use crate::ClientSubject;

    #[test]
    fn local_subject_conversion_preserves_static_and_owned_storage() {
        assert!(matches!(
            LocalSubject::from(ClientSubject::Static("users.v1.get")),
            LocalSubject::Static("users.v1.get")
        ));

        let owned = "users.v1.get.42".to_owned();
        let allocation = owned.as_ptr();
        let LocalSubject::Owned(converted) = LocalSubject::from(ClientSubject::Owned(owned)) else {
            panic!("owned client subject should stay owned");
        };
        assert_eq!(converted.as_ptr(), allocation);
    }
}
