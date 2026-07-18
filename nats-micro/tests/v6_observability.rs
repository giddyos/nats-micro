#![cfg(all(feature = "telemetry", feature = "live-test"))]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nats_micro::{
    AppConfig, Bytes, ClientError, ConsumerAction, HandlerPanicPolicy, MetricEvent, MetricName,
    Profile, TelemetryLayer, message, service, service_error, testing::LiveTestApp,
};

#[message]
pub struct ObservabilityResponse {
    pub value: String,
}

#[service_error]
pub enum ObservabilityError {
    #[error(code = 409, message = "conflict")]
    Conflict,
}

#[service(name = "v6-observability", version = "1.2.3")]
impl ObservabilityService {
    #[request("ok")]
    async fn ok() -> ObservabilityResponse {
        ObservabilityResponse {
            value: "ok".to_owned(),
        }
    }

    #[request("fail")]
    async fn fail() -> Result<ObservabilityResponse, ObservabilityError> {
        Err(ObservabilityError::Conflict)
    }

    #[consumer(
        stream = "V6_OBSERVABILITY",
        durable = "v6-observability-nack",
        filter = "v6.observability.nack",
        max_deliver = 2,
        ack_wait = "250ms"
    )]
    async fn nack() -> ConsumerAction {
        ConsumerAction::Nack
    }

    #[consumer(
        stream = "V6_OBSERVABILITY",
        durable = "v6-observability-term",
        filter = "v6.observability.term"
    )]
    async fn term() -> ConsumerAction {
        ConsumerAction::Term
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedEvent {
    metric: MetricName,
    value: i64,
    service_name: Option<String>,
    service_version: Option<String>,
    operation: Option<String>,
    subject_pattern: Option<String>,
    error_kind: Option<String>,
}

#[derive(Clone, Default)]
struct RecordingLayer {
    events: Arc<Mutex<Vec<RecordedEvent>>>,
}

impl TelemetryLayer for RecordingLayer {
    fn record(&self, event: MetricEvent<'_>) {
        self.events.lock().expect("event lock").push(RecordedEvent {
            metric: event.metric,
            value: event.value,
            service_name: event.service_name.map(str::to_owned),
            service_version: event.service_version.map(str::to_owned),
            operation: event.operation.map(str::to_owned),
            subject_pattern: event.subject_pattern.map(str::to_owned),
            error_kind: event.error_kind.map(str::to_owned),
        });
    }
}

#[nats_micro::live_test]
async fn static_workers_emit_layered_metrics_with_stable_labels() -> nats_micro::Result<()> {
    let layer = RecordingLayer::default();
    let mut config = AppConfig::for_profile(Profile::Test);
    config.telemetry.counters = true;
    config.telemetry.gauges = true;
    config.telemetry.histograms = true;

    let mut live = LiveTestApp::new()
        .config(config)
        .telemetry_layer(layer.clone())
        .serve(ObservabilityService)
        .start()
        .await?;
    let client = live.client::<ObservabilityService>();
    assert_eq!(client.ok().await?.value, "ok");
    assert!(matches!(
        client.fail().await,
        Err(ClientError::Service { .. })
    ));
    live.nats_client()
        .publish("v6.observability.nack", Bytes::from_static(b"nack"))
        .await?;
    live.nats_client()
        .publish("v6.observability.term", Bytes::from_static(b"term"))
        .await?;
    live.nats_client().flush().await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let complete = {
                let events = layer.events.lock().expect("event lock");
                [
                    MetricName::ConsumerDeliveries,
                    MetricName::ConsumerRedeliveries,
                    MetricName::ConsumerAckLatency,
                    MetricName::ConsumerNacks,
                    MetricName::ConsumerTerms,
                ]
                .iter()
                .all(|metric| events.iter().any(|event| event.metric == *metric))
            };
            if complete {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    live.server_mut().restart();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client.ok().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    live.shutdown().await?;

    let events = layer.events.lock().expect("event lock");
    for metric in [
        MetricName::RequestsReceived,
        MetricName::RequestsCompleted,
        MetricName::ServiceErrors,
        MetricName::RequestsInFlight,
        MetricName::EndpointLatency,
        MetricName::ConsumerDeliveries,
        MetricName::ConsumerRedeliveries,
        MetricName::ConsumerAckLatency,
        MetricName::ConsumerNacks,
        MetricName::ConsumerTerms,
        MetricName::Reconnects,
        MetricName::ShutdownDrainDuration,
    ] {
        assert!(
            events.iter().any(|event| event.metric == metric),
            "missing {}",
            metric.as_str()
        );
    }
    assert!(events.iter().any(|event| {
        event.service_name.as_deref() == Some("v6-observability")
            && event.service_version.as_deref() == Some("1.2.3")
            && event.operation.as_deref() == Some("fail")
            && event.subject_pattern.as_deref() == Some("v6-observability.v1.fail")
            && event.error_kind.as_deref() == Some("CONFLICT")
    }));
    Ok(())
}

#[test]
fn maximum_throughput_has_no_implicit_observability_or_wrappers() {
    let config = AppConfig::for_profile(Profile::MaximumThroughput);
    assert!(!config.generate_missing_request_ids);
    assert!(!config.telemetry.counters);
    assert!(!config.telemetry.gauges);
    assert!(!config.telemetry.histograms);
    assert!(!config.telemetry.tracing);
    assert_eq!(config.handler_panic, HandlerPanicPolicy::Propagate);
}

#[test]
fn all_required_metric_names_are_stable() {
    let names = [
        MetricName::RequestsReceived,
        MetricName::RequestsCompleted,
        MetricName::ServiceErrors,
        MetricName::FrameworkErrors,
        MetricName::RequestsInFlight,
        MetricName::EndpointLatency,
        MetricName::RejectedOverCapacity,
        MetricName::ConsumerDeliveries,
        MetricName::ConsumerRedeliveries,
        MetricName::ConsumerAckLatency,
        MetricName::ConsumerNacks,
        MetricName::ConsumerTerms,
        MetricName::WorkerRestarts,
        MetricName::Reconnects,
        MetricName::ShutdownDrainDuration,
    ]
    .map(MetricName::as_str);
    assert_eq!(names.len(), 15);
    assert!(names.iter().all(|name| name.starts_with("nats_micro.")));
}
