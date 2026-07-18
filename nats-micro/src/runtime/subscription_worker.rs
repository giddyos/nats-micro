#![cfg_attr(not(feature = "telemetry"), allow(unused_variables))]

use std::{panic::AssertUnwindSafe, sync::Arc};

use futures_util::{FutureExt, StreamExt, future::Either, stream::FuturesUnordered};
use tokio::sync::watch;

use crate::{Request, ShutdownState, SubscriptionHandler};

/// Runs one concrete core-NATS subscription with bounded, in-worker
/// concurrency and no per-message task.
pub async fn run_subscription<S, H>(
    state: Arc<S>,
    subscriber: async_nats::Subscriber,
    concurrency: usize,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    H: SubscriptionHandler<S>,
{
    run_subscription_with_panic_policy::<S, H>(
        state,
        subscriber,
        concurrency,
        crate::HandlerPanicPolicy::FailWorker,
        shutdown,
    )
    .await
}

pub async fn run_subscription_with_panic_policy<S, H>(
    state: Arc<S>,
    mut subscriber: async_nats::Subscriber,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    H: SubscriptionHandler<S>,
{
    anyhow::ensure!(concurrency > 0, "subscription concurrency must be non-zero");

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
                        subscriber.unsubscribe().await?;
                    }
                }

                completed = in_flight.next(), if !in_flight.is_empty() => {
                    handle_completion(completed, panic_policy)?;
                }

                incoming = subscriber.next() => {
                    let message = incoming.ok_or_else(|| {
                        anyhow::anyhow!("subscription stream ended unexpectedly")
                    })?;
                    let processing = process_message::<S, H>(&state, message);
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
            }
        } else {
            tokio::select! {
                biased;

                changed = shutdown.changed(), if accepting => {
                    if changed.is_err() || shutdown.borrow().is_requested() {
                        accepting = false;
                        subscriber.unsubscribe().await?;
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
            crate::trace::error!("subscription handler panicked; continuing by policy");
            Ok(())
        }
        Some(Err(_)) => Err(anyhow::anyhow!("subscription handler panicked")),
        None => Ok(()),
    }
}

async fn process_message<S, H>(state: &S, message: async_nats::Message) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    H: SubscriptionHandler<S>,
{
    let request = Request::new(
        message.subject.as_str(),
        message.reply.as_ref().map(async_nats::Subject::as_str),
        message.payload.as_ref(),
        message.headers.as_ref(),
    );
    H::call(state, request)
        .await
        .map_err(|error| anyhow::anyhow!("{}: {}", error.kind, error.message))
}
