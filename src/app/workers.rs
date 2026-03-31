use std::sync::Arc;

use async_nats::{
    error,
    jetstream::{self, AckKind},
    service::{self, endpoint},
};
use bytes::Bytes;
use futures::StreamExt;
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::{
    consumer::ConsumerDefinition,
    error::{IntoNatsError, NatsErrorResponse},
    handler::RequestContext,
    request::NatsRequest,
    service::EndpointDefinition,
    shutdown_signal::{ShutdownSignal, ShutdownState},
    utils::ensure_request_id,
};

use super::{
    NatsApp,
    limits::{resolve_endpoint_concurrency_limit, semaphore_permits},
    shutdown::shutdown_requested,
};

pub fn success_headers(success: bool) -> async_nats::HeaderMap {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert(
        crate::response::X_SUCCESS_HEADER,
        if success { "true" } else { "false" },
    );
    headers
}

async fn respond_payload(
    raw_req: &service::Request,
    success: bool,
    response: crate::NatsResponse,
) -> Result<(), async_nats::PublishError> {
    let mut headers = success_headers(success);
    for (name, values) in response.headers.iter() {
        let name_str: &str = name.as_ref();
        for value in values {
            headers.insert(name_str, value.as_str());
        }
    }

    raw_req
        .respond_with_headers(Ok(response.payload), headers)
        .await
}

async fn respond_error(
    raw_req: &service::Request,
    err: &NatsErrorResponse,
) -> Result<(), async_nats::PublishError> {
    let payload = serde_json::to_vec(err).unwrap_or_default();
    respond_payload(raw_req, false, crate::NatsResponse::new(payload)).await
}

async fn handle_endpoint_request(
    app: NatsApp,
    endpoint_def: EndpointDefinition,
    service_name: String,
    raw_req: service::Request,
    shutdown_signal: Option<ShutdownSignal>,
) {
    let headers: crate::Headers = raw_req.message.headers.clone().unwrap_or_default().into();
    let request_id = ensure_request_id(&headers);

    debug!(
        service = %service_name,
        group = %endpoint_def.group,
        subject = %raw_req.message.subject,
        request_id = %request_id,
        "received endpoint request"
    );

    let req = NatsRequest {
        subject: raw_req.message.subject.to_string(),
        payload: raw_req.message.payload.clone(),
        headers,
        reply: raw_req.message.reply.as_ref().map(|s| s.to_string()),
        request_id: request_id.clone(),
    };

    let prepared = match app.prepare_request_for_dispatch(req) {
        Ok(prepared) => prepared,
        Err(err) => {
            debug!(
                service = %service_name,
                group = %endpoint_def.group,
                subject = %raw_req.message.subject,
                request_id = %request_id,
                "request decryption failed"
            );
            let _ = respond_error(&raw_req, &err).await;
            return;
        }
    };

    let req = prepared.request;
    let request_subject = req.subject.clone();

    let mut ctx = RequestContext::new(req, app.state.clone(), endpoint_def.full_subject_template());
    if let Some(shutdown_signal) = shutdown_signal {
        ctx = ctx.__with_shutdown_signal(shutdown_signal);
    }
    #[cfg(feature = "encryption")]
    {
        ctx = ctx.__with_ephemeral_pub(prepared.ephemeral_pub);
    }

    let response = match endpoint_def.handler.call(ctx).await {
        Ok(res) => res,
        Err(err) => {
            error!(
                service = %service_name,
                group = %endpoint_def.group,
                subject = %request_subject,
                request_id = %request_id,
                error = %err,
                "endpoint handler failed"
            );
            let _ = respond_error(&raw_req, &err).await;
            return;
        }
    };

    debug!(
        service = %service_name,
        group = %endpoint_def.group,
        subject = %request_subject,
        request_id = %request_id,
        "sending endpoint response"
    );

    if let Err(err) = respond_payload(&raw_req, true, response).await {
        error!(
            service = %service_name,
            group = %endpoint_def.group,
            subject = %request_subject,
            request_id = %request_id,
            error = %err,
            "failed to respond"
        );
    }
}

pub(super) async fn run_endpoint_worker(
    app: NatsApp,
    endpoint_def: EndpointDefinition,
    service_name: String,
    mut endpoint_stream: endpoint::Endpoint,
    concurrency_limit: u64,
    mut shutdown_rx: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()> {
    let full_subject = endpoint_def.full_subject();
    let requires_shutdown_signal = endpoint_def.handler.requires_shutdown_signal();
    let semaphore = Arc::new(Semaphore::new(semaphore_permits(concurrency_limit)));
    let mut tasks = JoinSet::new();
    let mut should_stop_endpoint = false;

    info!(
        service = %service_name,
        group = %endpoint_def.group,
        subject = %endpoint_def.subject,
        full_subject = %full_subject,
        concurrency_limit = concurrency_limit,
        "endpoint worker started"
    );

    let outcome = loop {
        let permit = tokio::select! {
            biased;

            changed = shutdown_rx.changed() => {
                if shutdown_requested(changed, &shutdown_rx) {
                    should_stop_endpoint = true;
                    break Ok(());
                }
                continue;
            }
            join_result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(err)) = join_result {
                    error!(
                        service = %service_name,
                        group = %endpoint_def.group,
                        subject = %endpoint_def.subject,
                        full_subject = %full_subject,
                        error = %err,
                        "endpoint task panicked"
                    );
                }
                continue;
            }
            permit_result = semaphore.clone().acquire_owned() => match permit_result {
                Ok(permit) => permit,
                Err(_) => break Err(anyhow::anyhow!("endpoint semaphore closed unexpectedly")),
            },
        };

        let raw_req = tokio::select! {
            biased;

            changed = shutdown_rx.changed() => {
                drop(permit);
                if shutdown_requested(changed, &shutdown_rx) {
                    should_stop_endpoint = true;
                    break Ok(());
                }
                continue;
            }
            raw_req = endpoint_stream.next() => raw_req,
        };

        let Some(raw_req) = raw_req else {
            drop(permit);
            break Err(anyhow::anyhow!("endpoint subscription ended unexpectedly"));
        };

        let app = app.clone();
        let endpoint_def = endpoint_def.clone();
        let service_name = service_name.clone();
        let shutdown_signal =
            requires_shutdown_signal.then(|| ShutdownSignal::new(shutdown_rx.clone()));
        tasks.spawn(async move {
            let _permit = permit;
            handle_endpoint_request(app, endpoint_def, service_name, raw_req, shutdown_signal)
                .await;
        });
    };

    if should_stop_endpoint {
        debug!(
            service = %service_name,
            group = %endpoint_def.group,
            subject = %endpoint_def.subject,
            full_subject = %full_subject,
            "stopping endpoint subscription"
        );
        if let Err(err) = endpoint_stream.stop().await {
            debug!(
                service = %service_name,
                group = %endpoint_def.group,
                subject = %endpoint_def.subject,
                full_subject = %full_subject,
                error = %err,
                "endpoint subscription stop returned an error"
            );
        }
    }

    while let Some(join_result) = tasks.join_next().await {
        if let Err(err) = join_result {
            error!(
                service = %service_name,
                group = %endpoint_def.group,
                subject = %endpoint_def.subject,
                full_subject = %full_subject,
                error = %err,
                "endpoint task panicked"
            );
        }
    }

    info!(
        service = %service_name,
        group = %endpoint_def.group,
        subject = %endpoint_def.subject,
        full_subject = %full_subject,
        "endpoint worker stopped"
    );

    outcome
}

async fn handle_consumer_message(
    app: NatsApp,
    consumer_def: ConsumerDefinition,
    service_name: String,
    durable: String,
    message: jetstream::Message,
    shutdown_signal: Option<ShutdownSignal>,
) {
    let headers: crate::Headers = message.headers.clone().unwrap_or_default().into();
    let request_id = ensure_request_id(&headers);

    debug!(
        service = %service_name,
        consumer = %durable,
        subject = %message.subject,
        request_id = %request_id,
        "received consumer message"
    );

    let req = NatsRequest {
        subject: message.subject.to_string(),
        payload: message.payload.clone(),
        headers,
        reply: message.reply.clone().map(|s| s.to_string()),
        request_id: request_id.clone(),
    };

    let prepared = match app.prepare_request_for_dispatch(req) {
        Ok(prepared) => prepared,
        Err(err) => {
            error!(
                service = %service_name,
                consumer = %durable,
                request_id = %request_id,
                error = %err,
                "consumer request decryption failed"
            );
            let _ = message.ack_with(AckKind::Nak(None)).await;
            return;
        }
    };

    let req = prepared.request;

    let mut ctx = RequestContext::new(req, app.state.clone(), None);
    if let Some(shutdown_signal) = shutdown_signal {
        ctx = ctx.__with_shutdown_signal(shutdown_signal);
    }
    #[cfg(feature = "encryption")]
    {
        ctx = ctx.__with_ephemeral_pub(prepared.ephemeral_pub);
    }

    // Best-effort: periodically send Progress acks while the handler is running
    // to avoid redelivery mid-execution. Uses 1/3rd of the configured ack_wait.
    let ack_wait = consumer_def.config.ack_wait;
    let cancel = CancellationToken::new();

    let progress_handle = if !ack_wait.is_zero() {
        let tick = ack_wait
            .checked_div(3)
            .filter(|d| !d.is_zero())
            .unwrap_or_else(|| Duration::from_millis(100));

        let token = cancel.clone();
        let msg = message.clone();

        let service_name = service_name.clone();
        let durable = durable.clone();
        let request_id = request_id.clone();

        Some(tokio::spawn(async move {
            let mut next = Instant::now() + tick;

            loop {
                tokio::select! {
                    biased;

                    _ = token.cancelled() => break,

                    _ = tokio::time::sleep_until(next) => {
                        next += tick;

                        if msg.ack_with(AckKind::Progress).await.is_err() {
                            error!(
                                service = %service_name,
                                consumer = %durable,
                                subject = %msg.subject,
                                request_id = %request_id,
                                "failed to send Progress ack for consumer message"
                            );
                        }
                    }
                }
            }
        }))
    } else {
        None
    };

    let result = consumer_def.handler.call(ctx).await;

    cancel.cancel();
    if let Some(handle) = progress_handle {
        let _ = handle.await;
    }

    match result {
        Ok(_) => {
            debug!(
                service = %service_name,
                consumer = %durable,
                request_id = %request_id,
                "acking consumer message"
            );
            if let Err(err) = message.ack().await {
                error!(
                    service = %service_name,
                    error = %err,
                    consumer = %durable,
                    request_id = %request_id,
                    "failed to ack consumer message"
                );
            }
        }
        Err(err) => {
            error!(
                service = %service_name,
                error = %err,
                consumer = %durable,
                request_id = %request_id,
                "consumer handler failed"
            );
            debug!(
                service = %service_name,
                consumer = %durable,
                request_id = %request_id,
                "nacking consumer message"
            );
            let _ = message.ack_with(AckKind::Nak(None)).await;
        }
    }
}

pub(super) async fn run_consumer_worker(
    app: NatsApp,
    consumer_def: ConsumerDefinition,
    service_name: String,
    durable: String,
    mut messages: async_nats::jetstream::consumer::push::Messages,
    concurrency_limit: u64,
    mut shutdown_rx: watch::Receiver<ShutdownState>,
) -> anyhow::Result<()> {
    let requires_shutdown_signal = consumer_def.handler.requires_shutdown_signal();
    let semaphore = Arc::new(Semaphore::new(semaphore_permits(concurrency_limit)));
    let mut tasks = JoinSet::new();

    info!(
        service = %service_name,
        stream = %consumer_def.stream,
        durable = %durable,
        concurrency_limit = concurrency_limit,
        "consumer worker started"
    );

    let outcome = loop {
        let permit = tokio::select! {
            biased;

            changed = shutdown_rx.changed() => {
                if shutdown_requested(changed, &shutdown_rx) {
                    break Ok(());
                }
                continue;
            }
            join_result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(err)) = join_result {
                    error!(
                        service = %service_name,
                        stream = %consumer_def.stream,
                        durable = %durable,
                        error = %err,
                        "consumer task panicked"
                    );
                }
                continue;
            }
            permit_result = semaphore.clone().acquire_owned() => match permit_result {
                Ok(permit) => permit,
                Err(_) => break Err(anyhow::anyhow!("consumer semaphore closed unexpectedly")),
            },
        };

        let message = tokio::select! {
            biased;

            changed = shutdown_rx.changed() => {
                drop(permit);
                if shutdown_requested(changed, &shutdown_rx) {
                    break Ok(());
                }
                continue;
            }
            message = messages.next() => message,
        };

        let Some(message) = message else {
            drop(permit);
            break Err(anyhow::anyhow!(
                "consumer message stream ended unexpectedly"
            ));
        };

        let message = match message {
            Ok(message) => message,
            Err(err) => {
                drop(permit);
                error!(
                    service = %service_name,
                    consumer = %durable,
                    error = %err,
                    "failed to receive consumer message"
                );
                continue;
            }
        };

        let app = app.clone();
        let consumer_def = consumer_def.clone();
        let service_name = service_name.clone();
        let durable = durable.clone();
        let shutdown_signal =
            requires_shutdown_signal.then(|| ShutdownSignal::new(shutdown_rx.clone()));
        tasks.spawn(async move {
            let _permit = permit;
            handle_consumer_message(
                app,
                consumer_def,
                service_name,
                durable,
                message,
                shutdown_signal,
            )
            .await;
        });
    };

    while let Some(join_result) = tasks.join_next().await {
        if let Err(err) = join_result {
            error!(
                service = %service_name,
                stream = %consumer_def.stream,
                durable = %durable,
                error = %err,
                "consumer task panicked"
            );
        }
    }

    info!(
        service = %service_name,
        stream = %consumer_def.stream,
        durable = %durable,
        "consumer worker stopped"
    );

    outcome
}
