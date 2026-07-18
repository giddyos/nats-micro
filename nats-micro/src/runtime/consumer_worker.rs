use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use async_nats::jetstream::{self, AckKind};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::sync::watch;

use crate::{ConsumerAction, ConsumerHandler, Request, ShutdownState};

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
    mut messages: jetstream::consumer::push::Messages,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
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
                            in_flight.push(
                                AssertUnwindSafe(process_consumer::<S, C>(&state, message))
                                    .catch_unwind()
                            );
                        }
                        Some(Err(error)) => {
                            tracing::error!(%error, "failed to receive consumer message");
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
            tracing::error!("consumer handler panicked; continuing by policy");
            Ok(())
        }
        Some(Err(_)) => Err(anyhow::anyhow!("consumer handler panicked")),
        None => Ok(()),
    }
}

async fn process_consumer<S, C>(state: &S, message: jetstream::Message) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    C: ConsumerHandler<S>,
{
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
                tracing::error!(
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
            message
                .ack_with(AckKind::Nak(None))
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        ConsumerAction::NackAfter(delay) => {
            message
                .ack_with(AckKind::Nak(Some(delay)))
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        ConsumerAction::Term => {
            message
                .ack_with(AckKind::Term)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
    }

    Ok(())
}
