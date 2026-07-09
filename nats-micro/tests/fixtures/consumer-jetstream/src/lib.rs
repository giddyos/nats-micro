#![allow(clippy::unused_async)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nats_micro::{
    Auth, AuthError, AuthPolicy, FromAuthRequest, Headers, NatsErrorResponse, Payload, RequestId,
    RequestContext, State, service, service_handlers,
};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct ConsumerProbe {
    hits: Arc<AtomicUsize>,
    processed: Arc<Notify>,
}

impl ConsumerProbe {
    fn record(&self) {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.processed.notify_waiters();
    }
}

pub struct ConsumerClaims;

impl FromAuthRequest for ConsumerClaims {
    async fn from_auth_request(_ctx: &RequestContext) -> Result<Self, AuthError> {
        Ok(Self)
    }
}

#[service(name = "consumer-fixture", version = "1.0.0")]
pub struct ConsumerService;

#[service_handlers]
impl ConsumerService {
    #[consumer(stream = "FIXTURE_STREAM", durable = "fixture-durable")]
    async fn process_events(
        payload: Payload<String>,
        probe: State<ConsumerProbe>,
        _headers: Headers,
        _request_id: RequestId,
    ) -> Result<(), NatsErrorResponse> {
        assert_eq!(payload.into_inner(), "hello");
        probe.record();
        Ok(())
    }

    #[consumer(
        stream = "FIXTURE_STREAM",
        durable = "fixture-configured",
        config = nats_micro::NatsConsumerConfig {
            ack_wait: std::time::Duration::from_secs(1),
            ..Default::default()
        },
        concurrency_limit = 2
    )]
    async fn configured_events(
        _payload: Payload<String>,
        _auth: Option<Auth<ConsumerClaims>>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

pub fn assert_consumer_metadata() {
    let _alias_config: nats_micro::ConsumerConfig = nats_micro::NatsConsumerConfig::default();
    let _facade_config: nats_micro::NatsConsumerConfig = nats_micro::ConsumerConfig::default();

    let contract = ConsumerService::contract();
    assert_eq!(contract.consumers.len(), 2);
    assert_eq!(contract.consumers[0].auth_policy, AuthPolicy::None);
    assert_eq!(contract.consumers[1].auth_policy, AuthPolicy::Optional);
    assert_eq!(contract.consumers[1].concurrency_limit, Some(2));
}

#[cfg(test)]
mod tests {
    use std::{future::pending, time::Duration};

    use anyhow::{Context, Result};
    use nats_micro::{
        ConsumerDefinition, NatsApp, ServiceDefinition, ServiceMetadata, async_nats,
    };
    use tokio::{task::JoinHandle, time::timeout};

    use super::*;

    fn spawn_app(app: NatsApp) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            app.run_until(async {
                pending::<()>().await;
                Ok(())
            })
            .await
        })
    }

    fn consumer_service(consumer: ConsumerDefinition) -> ServiceDefinition {
        ServiceDefinition {
            metadata: ServiceMetadata::new("consumer-fixture-runtime", "1.0.0", "", None),
            endpoints: Vec::new(),
            consumers: vec![consumer],
            endpoint_info: Vec::new(),
            consumer_info: Vec::new(),
        }
    }

    #[tokio::test]
    async fn jetstream_consumer_processes_message() -> Result<()> {
        assert_consumer_metadata();

        let config_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../nats-server/configs/jetstream.conf");
        let server = nats_server::run_server(
            config_path
                .to_str()
                .expect("fixture JetStream config path should be UTF-8"),
        );
        let connected = nats_micro::connect(server.client_url(), None).await?;
        let jetstream = async_nats::jetstream::new(connected.client.clone());

        let stream_name = "FIXTURE_STREAM".to_string();
        let subject = "fixtures.events".to_string();
        let _stream = jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: stream_name.clone(),
                subjects: vec![subject.clone()],
                ..Default::default()
            })
            .await
            .context("failed to create fixture stream")?;

        let probe = ConsumerProbe::default();
        let app = NatsApp::new(connected.client.clone())
            .state(probe.clone())
            .service_def(consumer_service(ConsumerService::process_events_consumer()));
        let app_handle = spawn_app(app);

        jetstream
            .publish(subject.clone(), "hello".into())
            .await
            .context("failed to publish fixture message")?
            .await
            .context("failed to await fixture publish ack")?;

        timeout(Duration::from_secs(5), probe.processed.notified())
            .await
            .context("timed out waiting for fixture consumer")?;
        assert_eq!(probe.hits.load(Ordering::SeqCst), 1);

        connected.client.drain().await?;
        let app_result = timeout(Duration::from_secs(5), app_handle)
            .await
            .context("timed out waiting for consumer app shutdown")?
            .context("consumer app task failed")?;
        assert!(app_result.is_err());

        let _ = jetstream.delete_stream(&stream_name).await;
        Ok(())
    }
}
