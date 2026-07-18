#![cfg_attr(not(feature = "telemetry"), allow(unused_variables))]

use std::{future::Future, marker::PhantomData, panic::AssertUnwindSafe, sync::Arc};

use async_nats::service::{self, endpoint};
use futures_util::{FutureExt, StreamExt, future::Either, stream::FuturesUnordered};
use tokio::sync::watch;
#[cfg(feature = "telemetry")]
use tracing::Instrument;

use crate::{ErrorReply, Request, RequestEndpoint, Response, ShutdownState};
#[cfg(feature = "telemetry")]
use crate::{MetricName, telemetry::Telemetry};

#[cfg(feature = "encryption")]
type EndpointEncryption = Option<Arc<crate::ServiceKeyPair>>;

#[derive(Clone)]
struct DispatchContext<'a, S> {
    state: &'a S,
    #[cfg(feature = "encryption")]
    keypair: Option<&'a crate::ServiceKeyPair>,
    #[cfg(feature = "telemetry")]
    telemetry: Option<Telemetry>,
}

trait EndpointDispatch<S>: Send + Sync + 'static {
    #[cfg(feature = "telemetry")]
    const SPEC: crate::OperationSpec;

    fn process(
        context: DispatchContext<'_, S>,
        raw: service::Request,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + '_;
}

struct PlainEndpoint<E>(PhantomData<E>);

impl<S, E> EndpointDispatch<S> for PlainEndpoint<E>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    #[cfg(feature = "telemetry")]
    const SPEC: crate::OperationSpec = E::SPEC;

    async fn process(context: DispatchContext<'_, S>, raw: service::Request) -> anyhow::Result<()> {
        process_plain_request::<S, E>(
            context.state,
            raw,
            #[cfg(feature = "telemetry")]
            context.telemetry.as_ref(),
        )
        .await
    }
}

#[cfg(feature = "encryption")]
struct EncryptedEndpoint<E>(PhantomData<E>);

#[cfg(feature = "encryption")]
impl<S, E> EndpointDispatch<S> for EncryptedEndpoint<E>
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    #[cfg(feature = "telemetry")]
    const SPEC: crate::OperationSpec = E::SPEC;

    async fn process(context: DispatchContext<'_, S>, raw: service::Request) -> anyhow::Result<()> {
        process_encrypted_request::<S, E>(
            context.state,
            context
                .keypair
                .expect("encrypted endpoint startup requires a key pair"),
            raw,
            #[cfg(feature = "telemetry")]
            context.telemetry.as_ref(),
        )
        .await
    }
}

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
    endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    run_request_endpoint_inner::<S, PlainEndpoint<E>>(
        state,
        #[cfg(feature = "encryption")]
        EndpointEncryption::default(),
        endpoint,
        concurrency,
        panic_policy,
        #[cfg(feature = "telemetry")]
        None,
        shutdown,
    )
    .await
}

#[allow(clippy::too_many_lines)]
async fn run_request_endpoint_inner<S, D>(
    state: Arc<S>,
    #[cfg(feature = "encryption")] encryption: EndpointEncryption,
    mut endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    #[cfg(feature = "telemetry")] telemetry: Option<Telemetry>,
    mut shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    D: EndpointDispatch<S>,
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
                            crate::trace::debug!(%error, "endpoint stop returned an error");
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
                    #[cfg(feature = "telemetry")]
                    let span = if let Some(telemetry) = telemetry.as_ref() {
                        let subject = raw.message.subject.as_str();
                        let request_id = raw.message.headers.as_ref()
                            .and_then(|headers| headers.get("x-request-id"))
                            .map(async_nats::HeaderValue::as_str);
                        telemetry.operation(
                            MetricName::RequestsReceived,
                            1,
                            None,
                            &D::SPEC,
                            Some(subject),
                            request_id,
                            None,
                        );
                        telemetry.operation(
                            MetricName::RequestsInFlight,
                            1,
                            None,
                            &D::SPEC,
                            Some(subject),
                            request_id,
                            None,
                        );
                        if in_flight.len().saturating_add(1) >= concurrency {
                            telemetry.operation(
                                MetricName::RejectedOverCapacity,
                                1,
                                None,
                                &D::SPEC,
                                Some(subject),
                                request_id,
                                None,
                            );
                        }
                        telemetry.request_span(&D::SPEC, subject, request_id)
                    } else {
                        tracing::Span::none()
                    };
                    let context = DispatchContext {
                        state: state.as_ref(),
                        #[cfg(feature = "encryption")]
                        keypair: encryption.as_deref(),
                        #[cfg(feature = "telemetry")]
                        telemetry: telemetry.as_ref().map(Telemetry::fork),
                    };
                    #[cfg(feature = "telemetry")]
                    let telemetry_for_completion = telemetry.as_ref().map(Telemetry::fork);
                    let processing = async move {
                        #[cfg(feature = "telemetry")]
                        let started = std::time::Instant::now();
                        #[cfg(feature = "telemetry")]
                        let result = D::process(context, raw).instrument(span).await;
                        #[cfg(not(feature = "telemetry"))]
                        let result = D::process(context, raw).await;
                        #[cfg(feature = "telemetry")]
                        if let Some(telemetry) = telemetry_for_completion.as_ref() {
                            telemetry.operation(
                                MetricName::RequestsInFlight,
                                -1,
                                None,
                                &D::SPEC,
                                None,
                                None,
                                None,
                            );
                            telemetry.operation(
                                MetricName::EndpointLatency,
                                1,
                                Some(started.elapsed()),
                                &D::SPEC,
                                None,
                                None,
                                None,
                            );
                            let metric = if result.is_ok() {
                                MetricName::RequestsCompleted
                            } else {
                                MetricName::FrameworkErrors
                            };
                            telemetry.operation(
                                metric,
                                1,
                                None,
                                &D::SPEC,
                                None,
                                None,
                                result.as_ref().err().map(|_| "WORKER_PROCESS_FAILED"),
                            );
                        }
                        result
                    };
                    if panic_policy == crate::HandlerPanicPolicy::Propagate {
                        in_flight.push(Either::Left(async move { Ok(processing.await) }));
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
                        if let Err(error) = endpoint.stop().await {
                            crate::trace::debug!(%error, "endpoint stop returned an error");
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

#[cfg(feature = "telemetry")]
pub(crate) async fn run_request_endpoint_with_telemetry<S, E>(
    state: Arc<S>,
    endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    telemetry: Option<Telemetry>,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    run_request_endpoint_inner::<S, PlainEndpoint<E>>(
        state,
        #[cfg(feature = "encryption")]
        EndpointEncryption::default(),
        endpoint,
        concurrency,
        panic_policy,
        telemetry,
        shutdown,
    )
    .await
}

#[cfg(feature = "encryption")]
#[cfg_attr(feature = "telemetry", allow(dead_code))]
pub async fn run_encrypted_request_endpoint_with_panic_policy<S, E>(
    state: Arc<S>,
    keypair: Arc<crate::ServiceKeyPair>,
    endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    run_request_endpoint_inner::<S, EncryptedEndpoint<E>>(
        state,
        Some(keypair),
        endpoint,
        concurrency,
        panic_policy,
        #[cfg(feature = "telemetry")]
        None,
        shutdown,
    )
    .await
}

fn handle_completion(
    completed: Option<Result<anyhow::Result<()>, Box<dyn std::any::Any + Send>>>,
    panic_policy: crate::HandlerPanicPolicy,
) -> anyhow::Result<()> {
    match completed {
        Some(Ok(result)) => result,
        Some(Err(_)) if panic_policy == crate::HandlerPanicPolicy::LogAndContinue => {
            crate::trace::error!("request handler panicked; continuing by policy");
            Ok(())
        }
        Some(Err(_)) => Err(anyhow::anyhow!("request handler panicked")),
        None => Ok(()),
    }
}

#[cfg(all(feature = "encryption", feature = "telemetry"))]
pub(crate) async fn run_encrypted_request_endpoint_with_telemetry<S, E>(
    state: Arc<S>,
    keypair: Arc<crate::ServiceKeyPair>,
    endpoint: endpoint::Endpoint,
    concurrency: usize,
    panic_policy: crate::HandlerPanicPolicy,
    telemetry: Option<Telemetry>,
    shutdown: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    run_request_endpoint_inner::<S, EncryptedEndpoint<E>>(
        state,
        Some(keypair),
        endpoint,
        concurrency,
        panic_policy,
        telemetry,
        shutdown,
    )
    .await
}

async fn process_plain_request<S, E>(
    state: &S,
    raw: service::Request,
    #[cfg(feature = "telemetry")] telemetry: Option<&Telemetry>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: RequestEndpoint<S>,
{
    let message = &raw.message;
    #[cfg(feature = "telemetry")]
    let request_id = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get("x-request-id"))
        .map(async_nats::HeaderValue::as_str);
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
            #[cfg(feature = "telemetry")]
            record_endpoint_error(
                telemetry,
                &E::SPEC,
                message.subject.as_str(),
                request_id,
                &error,
            );
            respond_error(&raw, error).await?;
        }
    }

    Ok(())
}

#[cfg(feature = "encryption")]
async fn process_encrypted_request<S, E>(
    state: &S,
    keypair: &crate::ServiceKeyPair,
    raw: service::Request,
    #[cfg(feature = "telemetry")] telemetry: Option<&Telemetry>,
) -> anyhow::Result<()>
where
    S: Send + Sync + 'static,
    E: crate::EncryptedRequestEndpoint<S>,
{
    let message = &raw.message;
    #[cfg(feature = "telemetry")]
    let request_id = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get("x-request-id"))
        .map(async_nats::HeaderValue::as_str);
    let request = Request::new(
        message.subject.as_str(),
        message.reply.as_ref().map(async_nats::Subject::as_str),
        message.payload.as_ref(),
        message.headers.as_ref(),
    );

    match E::call(state, keypair, request).await {
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
            #[cfg(feature = "telemetry")]
            record_endpoint_error(
                telemetry,
                &E::SPEC,
                message.subject.as_str(),
                request_id,
                &error,
            );
            respond_error(&raw, error).await?;
        }
    }

    Ok(())
}

#[cfg(feature = "telemetry")]
fn record_endpoint_error(
    telemetry: Option<&Telemetry>,
    spec: &'static crate::OperationSpec,
    subject: &str,
    request_id: Option<&str>,
    error: &ErrorReply,
) {
    let Some(telemetry) = telemetry else {
        return;
    };
    let metric = if crate::FrameworkError::from_code(error.kind.as_ref()).is_some() {
        MetricName::FrameworkErrors
    } else {
        MetricName::ServiceErrors
    };
    telemetry.operation(
        metric,
        1,
        None,
        spec,
        Some(subject),
        request_id,
        Some(error.kind.as_ref()),
    );
}

async fn respond_error(
    raw: &service::Request,
    error: ErrorReply,
) -> Result<(), async_nats::PublishError> {
    let headers = crate::response::service_error_headers(&error);
    raw.respond_with_headers(Ok(error.payload), headers).await
}
