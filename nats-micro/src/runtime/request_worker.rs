use std::{panic::AssertUnwindSafe, sync::Arc};

use async_nats::service::{self, endpoint};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::sync::watch;

use crate::{ErrorReply, Request, RequestEndpoint, Response, ShutdownState};

/// Runs one concrete endpoint with bounded, in-worker concurrency.
///
/// The worker owns one application-state `Arc` and never clones it per request.
/// Handler futures remain inside the worker rather than becoming Tokio tasks.
pub async fn run_request_endpoint<S, E>(
    state: Arc<S>,
    endpoint: endpoint::Endpoint,
    concurrency: usize,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    run_request_endpoint_with_panic_policy::<S, E>(
        state,
        endpoint,
        concurrency,
        crate::HandlerPanicPolicy::FailWorker,
        shutdown,
    )
    .await
}

pub async fn run_request_endpoint_with_panic_policy<S, E>(
    state: Arc<S>,
    mut endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    anyhow::ensure!(concurrency > 0, "endpoint concurrency must be non-zero");

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
                        if let Err(error) = endpoint.stop().await {
                            tracing::debug!(%error, "endpoint stop returned an error");
                        }
                    }
                }

                completed = in_flight.next(), if !in_flight.is_empty() => {
                    handle_completion(completed, panic_policy)?;
                }

                incoming = endpoint.next() => {
                    let raw = incoming.ok_or_else(|| {
                        anyhow::anyhow!("endpoint stream ended unexpectedly")
                    })?;
                    in_flight.push(
                        AssertUnwindSafe(process_request::<S, E>(&state, raw)).catch_unwind()
                    );
                }
            }
        } else {
            tokio::select! {
                biased;

                changed = shutdown.changed(), if accepting => {
                    if changed.is_err() || shutdown.borrow().is_requested() {
                        accepting = false;
                        if let Err(error) = endpoint.stop().await {
                            tracing::debug!(%error, "endpoint stop returned an error");
                        }
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
            tracing::error!("request handler panicked; continuing by policy");
            Ok(())
        }
        Some(Err(_)) => Err(anyhow::anyhow!("request handler panicked")),
        None => Ok(()),
    }
}

async fn process_request<S, E>(state: &S, raw: service::Request) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    let message = &raw.message;
    let request = Request::new(
        message.subject.as_str(),
        message.reply.as_ref().map(async_nats::Subject::as_str),
        message.payload.as_ref(),
        message.headers.as_ref(),
    );

    match E::call(state, request).await {
        Ok(Response::Empty) => {
            raw.respond(Ok(bytes::Bytes::new())).await?;
        }
        Ok(Response::Payload(payload)) => {
            raw.respond(Ok(payload)).await?;
        }
        Ok(Response::WithHeaders { payload, headers }) => {
            raw.respond_with_headers(Ok(payload), headers).await?;
        }
        Err(error) => {
            respond_error(&raw, error).await?;
        }
    }

    Ok(())
}

async fn respond_error(
    raw: &service::Request,
    error: ErrorReply,
) -> Result<(), async_nats::PublishError> {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Nats-Service-Error", error.message.as_str());
    headers.insert("Nats-Service-Error-Code", error.code.to_string());
    headers.insert("Nats-Micro-Error-Kind", error.kind.as_ref());
    headers.insert("Nats-Micro-Error-Format", "json-v1");
    if let Some(request_id) = error.request_id.as_deref() {
        headers.insert("x-request-id", request_id);
    }
    raw.respond_with_headers(Ok(error.payload), headers).await
}
