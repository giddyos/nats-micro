#![cfg_attr(not(feature = "telemetry"), allow(unused_variables))]

use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use async_nats::jetstream::{self, AckKind};
use futures_util::{FutureExt, StreamExt, future::Either, stream::FuturesUnordered};
use tokio::sync::watch;
#[cfg(feature = "telemetry")]
use tracing::Instrument;

use crate::{ConsumerAction, ConsumerHandler, Request, ShutdownState};
#[cfg(feature = "telemetry")]
use crate::{MetricName, telemetry::Telemetry};

/// Runs one concrete `JetStream` consumer with bounded, in-worker concurrency.
///
/// Progress acknowledgements are polled alongside the handler future. The
/// message and application state are not cloned for dispatch.
pub async fn run_consumer<S, C>(
    state: Arc<S>,
    messages: jetstream::consumer::push::Messages,
    concurrency: usize,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    run_consumer_with_panic_policy::<S, C>(
        state,
        messages,
        concurrency,
        crate::HandlerPanicPolicy::FailWorker,
        shutdown,
    )
    .await
}

pub async fn run_consumer_with_panic_policy<S, C>(
    state: Arc<S>,
    messages: jetstream::consumer::push::Messages,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    run_consumer_inner::<S, C>(
        state,
        messages,
        concurrency,
        panic_policy,
        #[cfg(feature = "telemetry")]
        None,
        shutdown,
    )
    .await
}

#[cfg(feature = "telemetry")]
pub(crate) async fn run_consumer_with_telemetry<S, C>(
    state: Arc<S>,
    messages: jetstream::consumer::push::Messages,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    telemetry: Option<Telemetry>,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    run_consumer_inner::<S, C>(
        state,
        messages,
        concurrency,
        panic_policy,
        telemetry,
        shutdown,
    )
    .await
}

#[allow(clippy::too_many_lines)]
async fn run_consumer_inner<S, C>(
    state: Arc<S>,
    mut messages: jetstream::consumer::push::Messages,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    anyhow::ensure!(concurrency > 0, "consumer concurrency must be non-zero");

    let mut in_flight = FuturesUnordered::new();
    let mut accepting = true;

    loop {
        if !accepting && in_flight.is_empty() {
            return Ok(());
        }

        if accepting && in_flight.len() < concurrency {
            tokio::select! {
                biased;

                changed = shutdown.changed() => {
                    if changed.is_err() || shutdown.borrow().is_requested() {
                        accepting = false;
                    }
                }

                completed = in_flight.next(), if !in_flight.is_empty() => {
                    handle_completion(completed, panic_policy)?;
                }

                incoming = messages.next() => {
                    match incoming {
                        Some(Ok(message)) => {
                            #[cfg(feature = "telemetry")]
                            let span = if let Some(telemetry) = telemetry.as_ref() {
                                let subject = message.subject.as_str();
                                let request_id = message.headers.as_ref()
                                    .and_then(|headers| headers.get("x-request-id"))
                                    .map(async_nats::HeaderValue::as_str);
                                telemetry.consumer(
                                    MetricName::ConsumerDeliveries,
                                    1,
                                    None,
                                    &C::SPEC,
                                    Some(subject),
                                    request_id,
                                    None,
                                );
                                if message.info().is_ok_and(|info| info.delivered > 1) {
                                    telemetry.consumer(
                                        MetricName::ConsumerRedeliveries,
                                        1,
                                        None,
                                        &C::SPEC,
                                        Some(subject),
                                        request_id,
                                        None,
                                    );
                                }
                                telemetry.consumer_span(&C::SPEC, subject, request_id)
                            } else {
                                tracing::Span::none()
                            };
                            #[cfg(feature = "telemetry")]
                            let telemetry_for_message = telemetry.as_ref().map(Telemetry::fork);
                            let state_ref = state.as_ref();
                            let processing = async move {
                                #[cfg(feature = "telemetry")]
                                let result = process_consumer::<S, C>(
                                    state_ref,
                                    message,
                                    telemetry_for_message.as_ref(),
                                )
                                .instrument(span)
                                .await;
                                #[cfg(not(feature = "telemetry"))]
                                let result = process_consumer::<S, C>(state_ref, message).await;
                                result
                            };
                            if panic_policy == crate::HandlerPanicPolicy::Propagate {
                                in_flight.push(Either::Left(async move {
                                    Ok(processing.await)
                                }));
                            } else {
                                in_flight.push(Either::Right(
                                    AssertUnwindSafe(processing).catch_unwind()
                                ));
                            }
                        }
                        Some(Err(error)) => {
                            crate::trace::error!(%error, "failed to receive consumer message");
                        }
                        None => {
                            return Err(anyhow::anyhow!(
                                "consumer message stream ended unexpectedly"
                            ));
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                biased;

                changed = shutdown.changed(), if accepting => {
                    if changed.is_err() || shutdown.borrow().is_requested() {
                        accepting = false;
                    }
                }

                completed = in_flight.next(), if !in_flight.is_empty() => {
                    handle_completion(completed, panic_policy)?;
                }
            }
        }
    }
}

fn handle_completion(
    completed: Option<Result<anyhow::Result<()>, Box<dyn std::any::Any + Send>>>,
    panic_policy: crate::HandlerPanicPolicy,
) -> anyhow::Result<()> {
    match completed {
        Some(Ok(result)) => result,
        Some(Err(_)) if panic_policy == crate::HandlerPanicPolicy::LogAndContinue => {
            crate::trace::error!("consumer handler panicked; continuing by policy");
            Ok(())
        }
        Some(Err(_)) => Err(anyhow::anyhow!("consumer handler panicked")),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_lines)]
async fn process_consumer<S, C>(
    state: &S,
    message: jetstream::Message,
    #[cfg(feature = "telemetry")] telemetry: Option<&Telemetry>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
    #[cfg(feature = "telemetry")]
    let ack_started = std::time::Instant::now();
    #[cfg(feature = "telemetry")]
    let subject = message.subject.as_str();
    #[cfg(feature = "telemetry")]
    let request_id = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get("x-request-id"))
        .map(async_nats::HeaderValue::as_str);
    let action = {
        let request = Request::new(
            message.subject.as_str(),
            message.reply.as_ref().map(async_nats::Subject::as_str),
            message.payload.as_ref(),
            message.headers.as_ref(),
        );
        let handler = C::call(state, request);
        tokio::pin!(handler);

        let ack_wait = Duration::from_millis(C::SPEC.ack_wait_ms);
        let progress_period = ack_wait.checked_div(3).filter(|period| !period.is_zero());

        let result = if let Some(period) = progress_period {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;

            loop {
                tokio::select! {
                    biased;

                    result = &mut handler => break result,

                    _ = interval.tick() => {
                        message
                            .ack_with(AckKind::Progress)
                            .await
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                }
            }
        } else {
            handler.await
        };

        match result {
            Ok(action) => action,
            Err(error) => {
                #[cfg(feature = "telemetry")]
                if let Some(telemetry) = telemetry {
                    let metric = if crate::FrameworkError::from_code(error.kind.as_ref()).is_some()
                    {
                        MetricName::FrameworkErrors
                    } else {
                        MetricName::ServiceErrors
                    };
                    telemetry.consumer(
                        metric,
                        1,
                        None,
                        &C::SPEC,
                        Some(subject),
                        request_id,
                        Some(error.kind.as_ref()),
                    );
                }
                crate::trace::error!(
                    code = error.code,
                    kind = %error.kind,
                    message = %error.message,
                    "consumer handler failed"
                );
                ConsumerAction::Nack
            }
        }
    };

    match action {
        ConsumerAction::Ack => {
            message
                .ack()
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        ConsumerAction::Nack => {
            #[cfg(feature = "telemetry")]
            record_consumer_action(
                telemetry,
                MetricName::ConsumerNacks,
                &C::SPEC,
                subject,
                request_id,
            );
            message
                .ack_with(AckKind::Nak(None))
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        ConsumerAction::NackAfter(delay) => {
            #[cfg(feature = "telemetry")]
            record_consumer_action(
                telemetry,
                MetricName::ConsumerNacks,
                &C::SPEC,
                subject,
                request_id,
            );
            message
                .ack_with(AckKind::Nak(Some(delay)))
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        ConsumerAction::Term => {
            #[cfg(feature = "telemetry")]
            record_consumer_action(
                telemetry,
                MetricName::ConsumerTerms,
                &C::SPEC,
                subject,
                request_id,
            );
            message
                .ack_with(AckKind::Term)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
    }

    #[cfg(feature = "telemetry")]
    if let Some(telemetry) = telemetry {
        telemetry.consumer(
            MetricName::ConsumerAckLatency,
            1,
            Some(ack_started.elapsed()),
            &C::SPEC,
            Some(subject),
            request_id,
            None,
        );
    }

    Ok(())
}

#[cfg(feature = "telemetry")]
fn record_consumer_action(
    telemetry: Option<&Telemetry>,
    metric: MetricName,
    spec: &'static crate::ConsumerSpec,
    subject: &str,
    request_id: Option<&str>,
) {
    if let Some(telemetry) = telemetry {
        telemetry.consumer(metric, 1, None, spec, Some(subject), request_id, None);
    }
}
