use std::{future::Future, sync::Arc};

use async_nats::HeaderMap;
use bytes::Bytes;

pub use nats_micro_testing::{
    CapturedEvent, EventLog, EventSelection, FaultPlan, init_test_tracing,
};
#[cfg(feature = "test-jetstream")]
pub use nats_micro_testing::{
    JetStreamConfig, JetStreamSimulator, ManualClock, SimulatedAction, SimulatedConsumerConfig,
};

use crate::{
    ClientRequest, ClientResponse, ClientTransport, Cons, Nil, RequestEndpoint, Response, Service,
    ServiceSet,
};
#[cfg(feature = "test-jetstream")]
use crate::{ConsumerAction, ConsumerHandler};

#[cfg(feature = "live-test")]
mod live;
#[cfg(feature = "live-test")]
pub use live::{
    LiveServiceSet, LiveTestApp, LiveTestHarness, init_live_test_diagnostics, is_live_test_skip,
    live_test_timeout,
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

#[cfg(feature = "test-jetstream")]
#[derive(Debug)]
pub enum LocalConsumerDispatch {
    Matched(ConsumerAction),
    NotMatched(LocalRequest),
}

pub trait LocalServiceSet<S>: Send + Sync + 'static {
    #[cfg(feature = "encryption")]
    fn requires_encryption(&self) -> bool;

    fn dispatch<'a>(
        &'a self,
        state: &'a S,
        #[cfg(feature = "encryption")] keypair: Option<&'a crate::ServiceKeyPair>,
        request: LocalRequest,
    ) -> impl Future<Output = LocalDispatch> + Send + 'a;
}

impl<S> LocalServiceSet<S> for Nil
where
    S: Send + Sync + 'static,
{
    #[cfg(feature = "encryption")]
    fn requires_encryption(&self) -> bool {
        false
    }

    async fn dispatch<'a>(
        &'a self,
        _state: &'a S,
        #[cfg(feature = "encryption")] _keypair: Option<&'a crate::ServiceKeyPair>,
        request: LocalRequest,
    ) -> LocalDispatch {
        LocalDispatch::NotMatched(request)
    }
}

impl<S, Head, Tail> LocalServiceSet<S> for Cons<Head, Tail>
where
    S: Send + Sync + 'static,
    Head: Service<S>,
    Tail: LocalServiceSet<S>,
{
    #[cfg(feature = "encryption")]
    fn requires_encryption(&self) -> bool {
        Head::SPEC
            .operations
            .iter()
            .any(|operation| operation.request_encrypted || operation.response_encrypted)
            || self.tail.requires_encryption()
    }

    async fn dispatch<'a>(
        &'a self,
        state: &'a S,
        #[cfg(feature = "encryption")] keypair: Option<&'a crate::ServiceKeyPair>,
        request: LocalRequest,
    ) -> LocalDispatch {
        match self
            .head
            .dispatch_local(
                state,
                #[cfg(feature = "encryption")]
                keypair,
                request,
            )
            .await
        {
            LocalDispatch::Matched(response) => LocalDispatch::Matched(response),
            LocalDispatch::NotMatched(request) => {
                self.tail
                    .dispatch(
                        state,
                        #[cfg(feature = "encryption")]
                        keypair,
                        request,
                    )
                    .await
            }
        }
    }
}

#[cfg(feature = "test-jetstream")]
pub trait LocalConsumerServiceSet<S>: Send + Sync + 'static {
    fn register_consumers(&self, jetstream: &JetStreamSimulator) -> anyhow::Result<()>;

    fn dispatch_consumer<'a>(
        &'a self,
        state: &'a S,
        durable: &'a str,
        request: LocalRequest,
    ) -> impl Future<Output = LocalConsumerDispatch> + Send + 'a;
}

#[cfg(feature = "test-jetstream")]
impl<S> LocalConsumerServiceSet<S> for Nil
where
    S: Send + Sync + 'static,
{
    fn register_consumers(&self, _jetstream: &JetStreamSimulator) -> anyhow::Result<()> {
        Ok(())
    }

    async fn dispatch_consumer<'a>(
        &'a self,
        _state: &'a S,
        _durable: &'a str,
        request: LocalRequest,
    ) -> LocalConsumerDispatch {
        LocalConsumerDispatch::NotMatched(request)
    }
}

#[cfg(feature = "test-jetstream")]
impl<S, Head, Tail> LocalConsumerServiceSet<S> for Cons<Head, Tail>
where
    S: Send + Sync + 'static,
    Head: Service<S>,
    Tail: LocalConsumerServiceSet<S>,
{
    fn register_consumers(&self, jetstream: &JetStreamSimulator) -> anyhow::Result<()> {
        for consumer in Head::SPEC.consumers {
            jetstream.register_consumer(SimulatedConsumerConfig {
                stream: consumer.stream.to_owned(),
                durable: consumer.durable.to_owned(),
                filter_subject: consumer.filter_subject.to_owned(),
                ack_wait: std::time::Duration::from_millis(consumer.ack_wait_ms),
                max_deliver: u64::try_from(consumer.max_deliver)
                    .unwrap_or(u64::MAX)
                    .max(1),
                backoff: consumer
                    .backoff_ms
                    .iter()
                    .copied()
                    .map(std::time::Duration::from_millis)
                    .collect(),
            })?;
        }
        self.tail.register_consumers(jetstream)
    }

    async fn dispatch_consumer<'a>(
        &'a self,
        state: &'a S,
        durable: &'a str,
        request: LocalRequest,
    ) -> LocalConsumerDispatch {
        match self
            .head
            .dispatch_consumer_local(state, durable, request)
            .await
        {
            LocalConsumerDispatch::Matched(action) => LocalConsumerDispatch::Matched(action),
            LocalConsumerDispatch::NotMatched(request) => {
                self.tail.dispatch_consumer(state, durable, request).await
            }
        }
    }
}

struct Inner<S, Services> {
    state: S,
    services: Services,
    events: EventLog,
    faults: FaultPlan,
    #[cfg(feature = "test-jetstream")]
    jetstream: JetStreamSimulator,
    #[cfg(feature = "encryption")]
    encryption: Option<Arc<crate::ServiceKeyPair>>,
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
        let dispatch = self.inner.services.dispatch(
            &self.inner.state,
            #[cfg(feature = "encryption")]
            self.inner.encryption.as_deref(),
            request,
        );
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
    #[cfg(feature = "test-jetstream")]
    jetstream: JetStreamConfig,
    #[cfg(feature = "encryption")]
    encryption: Option<Arc<crate::ServiceKeyPair>>,
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
            #[cfg(feature = "test-jetstream")]
            jetstream: JetStreamConfig::default(),
            #[cfg(feature = "encryption")]
            encryption: None,
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
            #[cfg(feature = "test-jetstream")]
            jetstream: self.jetstream,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
        }
    }

    #[cfg(feature = "encryption")]
    #[must_use]
    pub fn encryption(mut self, keypair: crate::ServiceKeyPair) -> Self {
        self.encryption = Some(Arc::new(keypair));
        self
    }

    #[cfg(feature = "test-jetstream")]
    #[must_use]
    pub fn jetstream(mut self, configure: impl FnOnce(&mut JetStreamConfig)) -> Self {
        configure(&mut self.jetstream);
        self
    }

    #[cfg(not(feature = "test-jetstream"))]
    #[must_use]
    pub fn start(self) -> TestHarness<S, Services>
    where
        Services: LocalServiceSet<S> + ServiceSet<S>,
    {
        crate::validate_service_set::<S, Services>(&self.services)
            .expect("test application service validation failed");
        #[cfg(feature = "encryption")]
        assert!(
            !self.services.requires_encryption() || self.encryption.is_some(),
            "test application declares encrypted operations but has no encryption key"
        );
        let inner = Arc::new(Inner {
            state: self.state,
            services: self.services,
            events: self.events,
            faults: self.faults,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
        });
        TestHarness {
            transport: LocalTransport { inner },
        }
    }

    #[cfg(feature = "test-jetstream")]
    #[must_use]
    pub fn start(self) -> TestHarness<S, Services>
    where
        Services: LocalServiceSet<S> + LocalConsumerServiceSet<S> + ServiceSet<S>,
    {
        crate::validate_service_set::<S, Services>(&self.services)
            .expect("test application service validation failed");
        #[cfg(feature = "encryption")]
        assert!(
            !self.services.requires_encryption() || self.encryption.is_some(),
            "test application declares encrypted operations but has no encryption key"
        );
        let jetstream = JetStreamSimulator::new(self.jetstream, self.events.clone());
        self.services
            .register_consumers(&jetstream)
            .expect("simulated JetStream consumer validation failed");
        let inner = Arc::new(Inner {
            state: self.state,
            services: self.services,
            events: self.events,
            faults: self.faults,
            jetstream,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
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

    #[cfg(feature = "encryption")]
    #[must_use]
    pub fn encrypted_client<ServiceType>(&self) -> ServiceType::Client<LocalTransport<S, Services>>
    where
        ServiceType: crate::EncryptedService<S>,
    {
        let keypair = self
            .transport
            .inner
            .encryption
            .as_deref()
            .expect("encrypted test client requires a TestApp encryption key");
        ServiceType::encrypted_client(
            self.transport.clone(),
            crate::ServiceRecipient::from_bytes(keypair.public_key_bytes()),
        )
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

#[cfg(feature = "test-jetstream")]
impl<S, Services> TestHarness<S, Services>
where
    S: Send + Sync + 'static,
    Services: LocalServiceSet<S> + LocalConsumerServiceSet<S>,
{
    #[must_use]
    pub fn jetstream(&self) -> TestJetStream {
        TestJetStream {
            simulator: self.transport.inner.jetstream.clone(),
        }
    }

    #[must_use]
    pub fn clock(&self) -> ManualClock {
        self.transport.inner.jetstream.clock()
    }

    #[must_use]
    pub fn deliveries(&self, durable: &str) -> u64 {
        self.transport.inner.jetstream.deliveries(durable)
    }

    pub async fn run_until_idle(&self) -> anyhow::Result<()> {
        while let Some(delivery) = self.transport.inner.jetstream.next_due() {
            if self
                .transport
                .inner
                .jetstream
                .consume_timeout(delivery.durable())
            {
                self.transport.inner.jetstream.complete_timeout(delivery);
                continue;
            }

            let durable = delivery.durable().to_owned();
            let attempt = delivery.attempt();
            let (subject, payload, mut headers) = delivery.clone().into_parts();
            headers
                .get_or_insert_with(HeaderMap::new)
                .insert("Nats-Num-Delivered", attempt.to_string());
            let request = LocalRequest {
                subject: LocalSubject::Owned(subject),
                payload,
                headers,
            };
            let action = match self
                .transport
                .inner
                .services
                .dispatch_consumer(&self.transport.inner.state, &durable, request)
                .await
            {
                LocalConsumerDispatch::Matched(action) => action,
                LocalConsumerDispatch::NotMatched(_) => {
                    anyhow::bail!("no simulated consumer route matched durable `{durable}`");
                }
            };
            self.transport.inner.jetstream.complete(
                delivery,
                match action {
                    ConsumerAction::Ack => SimulatedAction::Ack,
                    ConsumerAction::Nack => SimulatedAction::Nack,
                    ConsumerAction::NackAfter(delay) => SimulatedAction::NackAfter(delay),
                    ConsumerAction::Term => SimulatedAction::Term,
                },
            );
        }
        Ok(())
    }
}

#[cfg(feature = "test-jetstream")]
/// Deterministic application-semantics simulator for generated consumers.
///
/// It intentionally does not model clustering, persistence, account
/// permissions, server-side stream configuration edge cases, or reconnect
/// behavior. Use `LiveTestApp` under `live-test` for those protocol concerns.
#[derive(Debug, Clone)]
pub struct TestJetStream {
    simulator: JetStreamSimulator,
}

#[cfg(feature = "test-jetstream")]
#[allow(clippy::unused_async)]
impl TestJetStream {
    pub async fn publish(&self, subject: &str, payload: impl Into<Bytes>) -> anyhow::Result<()> {
        self.simulator.publish(subject, &payload.into(), None)
    }

    pub async fn publish_json<T>(&self, subject: &str, value: &T) -> anyhow::Result<()>
    where
        T: serde::Serialize + ?Sized,
    {
        self.simulator
            .publish(subject, &Bytes::from(serde_json::to_vec(value)?), None)
    }

    pub async fn publish_proto<T>(&self, subject: &str, value: &T) -> anyhow::Result<()>
    where
        T: prost::Message,
    {
        self.simulator
            .publish(subject, &Bytes::from(value.encode_to_vec()), None)
    }

    pub fn timeout_next(&self, durable: impl Into<String>) {
        self.simulator.timeout_next(durable);
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        self.simulator.pending()
    }
}

pub async fn dispatch<S, E>(state: &S, request: LocalRequest) -> LocalDispatch
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    response_to_local_dispatch(E::call(state, request.view()).await)
}

fn response_to_local_dispatch(response: crate::DispatchResult) -> LocalDispatch {
    let response = match response {
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

#[cfg(feature = "encryption")]
pub async fn dispatch_encrypted<S, E>(
    state: &S,
    keypair: Option<&crate::ServiceKeyPair>,
    request: LocalRequest,
) -> LocalDispatch
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    let response = if let Some(keypair) = keypair {
        E::call(state, keypair, request.view()).await
    } else {
        Err(crate::ErrorReply::framework(
            crate::FrameworkError::NoEncryptionKey,
            "test application has no encryption key",
            request.view().request_id().existing(),
        ))
    };
    response_to_local_dispatch(response)
}

#[cfg(feature = "test-jetstream")]
pub async fn dispatch_consumer<S, C>(state: &S, request: LocalRequest) -> LocalConsumerDispatch
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    let action = C::call(state, request.view())
        .await
        .unwrap_or(ConsumerAction::Nack);
    LocalConsumerDispatch::Matched(action)
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
