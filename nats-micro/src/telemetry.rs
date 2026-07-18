use std::sync::Arc;
use std::time::Duration;

use crate::{ConsumerSpec, OperationSpec, TelemetryConfig};

/// Stable metric identifiers emitted by static runtime workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MetricName {
    RequestsReceived,
    RequestsCompleted,
    ServiceErrors,
    FrameworkErrors,
    RequestsInFlight,
    EndpointLatency,
    RejectedOverCapacity,
    ConsumerDeliveries,
    ConsumerRedeliveries,
    ConsumerAckLatency,
    ConsumerNacks,
    ConsumerTerms,
    WorkerRestarts,
    Reconnects,
    ShutdownDrainDuration,
}

impl MetricName {
    /// Returns the stable dotted metric name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestsReceived => "nats_micro.requests.received",
            Self::RequestsCompleted => "nats_micro.requests.completed",
            Self::ServiceErrors => "nats_micro.errors.service",
            Self::FrameworkErrors => "nats_micro.errors.framework",
            Self::RequestsInFlight => "nats_micro.requests.in_flight",
            Self::EndpointLatency => "nats_micro.endpoint.latency",
            Self::RejectedOverCapacity => "nats_micro.requests.rejected_over_capacity",
            Self::ConsumerDeliveries => "nats_micro.consumer.deliveries",
            Self::ConsumerRedeliveries => "nats_micro.consumer.redeliveries",
            Self::ConsumerAckLatency => "nats_micro.consumer.ack_latency",
            Self::ConsumerNacks => "nats_micro.consumer.nacks",
            Self::ConsumerTerms => "nats_micro.consumer.terms",
            Self::WorkerRestarts => "nats_micro.worker.restarts",
            Self::Reconnects => "nats_micro.connection.reconnects",
            Self::ShutdownDrainDuration => "nats_micro.shutdown.drain_duration",
        }
    }

    const fn enabled_by(self, config: TelemetryConfig) -> bool {
        match self {
            Self::EndpointLatency | Self::ConsumerAckLatency | Self::ShutdownDrainDuration => {
                config.histograms
            }
            Self::RequestsInFlight => config.gauges,
            Self::RequestsReceived
            | Self::RequestsCompleted
            | Self::ServiceErrors
            | Self::FrameworkErrors
            | Self::RejectedOverCapacity
            | Self::ConsumerDeliveries
            | Self::ConsumerRedeliveries
            | Self::ConsumerNacks
            | Self::ConsumerTerms
            | Self::WorkerRestarts
            | Self::Reconnects => config.counters,
        }
    }
}

/// One borrowed metric observation.
///
/// Static fields point into generated metadata and dynamic fields point into
/// the current NATS frame. Layers that retain observations must copy them.
#[derive(Debug, Clone, Copy)]
pub struct MetricEvent<'a> {
    pub metric: MetricName,
    pub value: i64,
    pub duration: Option<Duration>,
    pub service_name: Option<&'a str>,
    pub service_version: Option<&'a str>,
    pub operation: Option<&'a str>,
    pub subject_pattern: Option<&'a str>,
    pub queue_group: Option<&'a str>,
    pub stream: Option<&'a str>,
    pub consumer: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub request_id: Option<&'a str>,
    pub error_kind: Option<&'a str>,
}

impl<'a> MetricEvent<'a> {
    fn operation(
        metric: MetricName,
        value: i64,
        duration: Option<Duration>,
        spec: &'static OperationSpec,
        subject: Option<&'a str>,
        request_id: Option<&'a str>,
        error_kind: Option<&'a str>,
    ) -> Self {
        Self {
            metric,
            value,
            duration,
            service_name: Some(spec.service_name),
            service_version: Some(spec.service_version),
            operation: Some(spec.rust_name),
            subject_pattern: Some(spec.subject),
            queue_group: spec.queue_group,
            stream: None,
            consumer: None,
            subject,
            request_id,
            error_kind,
        }
    }

    fn consumer(
        metric: MetricName,
        value: i64,
        duration: Option<Duration>,
        spec: &'static ConsumerSpec,
        subject: Option<&'a str>,
        request_id: Option<&'a str>,
        error_kind: Option<&'a str>,
    ) -> Self {
        Self {
            metric,
            value,
            duration,
            service_name: Some(spec.service_name),
            service_version: Some(spec.service_version),
            operation: Some(spec.rust_name),
            subject_pattern: Some(spec.filter_subject),
            queue_group: None,
            stream: Some(spec.stream),
            consumer: Some(spec.durable),
            subject,
            request_id,
            error_kind,
        }
    }
}

/// Application-installed sink for runtime metrics.
pub trait TelemetryLayer: Send + Sync + 'static {
    fn record(&self, event: MetricEvent<'_>);
}

#[derive(Clone)]
pub(crate) struct Telemetry {
    layer: Arc<dyn TelemetryLayer>,
    config: TelemetryConfig,
}

impl Telemetry {
    pub(crate) fn new(layer: Arc<dyn TelemetryLayer>, config: TelemetryConfig) -> Self {
        Self { layer, config }
    }

    pub(crate) fn fork(&self) -> Self {
        Self {
            layer: Arc::clone(&self.layer),
            config: self.config,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn operation(
        &self,
        metric: MetricName,
        value: i64,
        duration: Option<Duration>,
        spec: &'static OperationSpec,
        subject: Option<&str>,
        request_id: Option<&str>,
        error_kind: Option<&str>,
    ) {
        if metric.enabled_by(self.config) {
            self.layer.record(MetricEvent::operation(
                metric, value, duration, spec, subject, request_id, error_kind,
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn consumer(
        &self,
        metric: MetricName,
        value: i64,
        duration: Option<Duration>,
        spec: &'static ConsumerSpec,
        subject: Option<&str>,
        request_id: Option<&str>,
        error_kind: Option<&str>,
    ) {
        if metric.enabled_by(self.config) {
            self.layer.record(MetricEvent::consumer(
                metric, value, duration, spec, subject, request_id, error_kind,
            ));
        }
    }

    pub(crate) fn application(&self, metric: MetricName, duration: Option<Duration>) {
        if metric.enabled_by(self.config) {
            self.layer.record(MetricEvent {
                metric,
                value: 1,
                duration,
                service_name: None,
                service_version: None,
                operation: None,
                subject_pattern: None,
                queue_group: None,
                stream: None,
                consumer: None,
                subject: None,
                request_id: None,
                error_kind: None,
            });
        }
    }

    pub(crate) fn request_span(
        &self,
        spec: &'static OperationSpec,
        subject: &str,
        request_id: Option<&str>,
    ) -> tracing::Span {
        if self.config.tracing && tracing::enabled!(tracing::Level::INFO) {
            tracing::info_span!(
                "nats.request",
                "service.name" = spec.service_name,
                "service.version" = spec.service_version,
                "nats.operation" = spec.rust_name,
                "nats.subject_pattern" = spec.subject,
                "nats.queue_group" = spec.queue_group,
                "nats.stream" = tracing::field::Empty,
                "nats.consumer" = tracing::field::Empty,
                "nats.subject" = subject,
                "x-request-id" = request_id,
            )
        } else {
            tracing::Span::none()
        }
    }

    pub(crate) fn consumer_span(
        &self,
        spec: &'static ConsumerSpec,
        subject: &str,
        request_id: Option<&str>,
    ) -> tracing::Span {
        if self.config.tracing && tracing::enabled!(tracing::Level::INFO) {
            tracing::info_span!(
                "nats.consumer",
                "service.name" = spec.service_name,
                "service.version" = spec.service_version,
                "nats.operation" = spec.rust_name,
                "nats.subject_pattern" = spec.filter_subject,
                "nats.queue_group" = tracing::field::Empty,
                "nats.stream" = spec.stream,
                "nats.consumer" = spec.durable,
                "nats.subject" = subject,
                "x-request-id" = request_id,
            )
        } else {
            tracing::Span::none()
        }
    }
}
