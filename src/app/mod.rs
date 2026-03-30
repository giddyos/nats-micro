mod limits;
mod prepare;
mod shutdown;
#[cfg(test)]
mod tests;
mod workers;

use std::{future::Future, sync::Arc};

use anyhow::Result;
use async_nats::jetstream;
use async_nats::service::ServiceExt;
use tokio::{sync::watch, task::JoinHandle};
use tracing::{debug, error, info};

use crate::{
    auth::AuthConfig, consumer::ConsumerDefinition, error::NatsErrorResponse, request::NatsRequest,
    service::NatsService, service::ServiceDefinition, state::StateMap, utils::has_auth_headers,
};

use self::{
    limits::{
        DEFAULT_CONCURRENCY_LIMIT, resolve_consumer_concurrency_limit,
        resolve_endpoint_concurrency_limit, validate_consumer_concurrency_limit,
    },
    prepare::{PreparedRequest, prepare_request_for_dispatch_with_state},
    shutdown::{LiveService, ShutdownHook, run_shutdown_hook},
    workers::{run_consumer_worker, run_endpoint_worker},
};

pub use self::workers::success_headers;

#[derive(Clone)]
pub struct NatsApp {
    client: async_nats::Client,
    state: StateMap,
    auth: Option<AuthConfig>,
    service_defs: Vec<ServiceDefinition>,
    shutdown_hook: Option<ShutdownHook>,
}

impl NatsApp {
    pub fn new(client: async_nats::Client) -> Self {
        Self {
            client,
            state: StateMap::new(),
            auth: None,
            service_defs: Vec::new(),
            shutdown_hook: None,
        }
    }

    pub fn state<T>(mut self, value: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.state = self.state.insert(value);
        self
    }

    pub fn with_auth<U, F, Fut>(mut self, resolver: F) -> Self
    where
        U: Send + Sync + 'static,
        F: Fn(&NatsRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<U, crate::auth::AuthError>> + Send + 'static,
    {
        self.auth = Some(AuthConfig::new::<U, _, _>(resolver));
        self
    }

    pub fn service<S: NatsService>(mut self) -> Self {
        self.service_defs.push(S::definition());
        self
    }

    #[cfg(feature = "encryption")]
    pub fn with_encryption(mut self, keypair: crate::encryption::ServiceKeyPair) -> Self {
        self.state = self.state.insert(keypair);
        self
    }

    pub fn service_def(mut self, def: ServiceDefinition) -> Self {
        self.service_defs.push(def);
        self
    }

    pub fn with_shutdown_hook<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.shutdown_hook = Some(Arc::new(move || Box::pin(hook())));
        self
    }

    pub async fn run(mut self) -> Result<()> {
        if self.service_defs.is_empty() {
            anyhow::bail!(
                "NatsApp requires at least one explicit service via .service(...) or .service_def(...)"
            );
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut live_services: Vec<LiveService> = Vec::new();
        let total_services = self.service_defs.len();

        info!(service_count = total_services, "starting nats application");

        for svc_def in std::mem::take(&mut self.service_defs) {
            info!(
                service = %svc_def.metadata.name,
                version = %svc_def.metadata.version,
                endpoint_count = svc_def.endpoints.len(),
                consumer_count = svc_def.consumers.len(),
                "spawning service"
            );
            let svc = match self.spawn_service(svc_def, shutdown_rx.clone()).await {
                Ok(svc) => svc,
                Err(err) => {
                    let _ = shutdown_tx.send(true);
                    for live_service in live_services {
                        live_service.shutdown().await;
                    }
                    return Err(err);
                }
            };
            live_services.push(svc);
        }

        info!(
            service_count = live_services.len(),
            "all services are running"
        );
        tokio::signal::ctrl_c().await?;
        info!(
            service_count = live_services.len(),
            "shutdown signal received"
        );
        let _ = shutdown_tx.send(true);

        let shutdown_hook_result = if self.shutdown_hook.is_some() {
            info!("running pre-shutdown hook");
            let result = run_shutdown_hook(self.shutdown_hook.as_ref()).await;
            if let Err(err) = &result {
                error!(error = %err, "pre-shutdown hook failed");
            }
            result
        } else {
            Ok(())
        };

        info!(
            service_count = live_services.len(),
            "waiting for workers to drain"
        );
        for live_service in live_services {
            live_service.shutdown().await;
        }

        shutdown_hook_result?;
        Ok(())
    }

    fn prepare_request_for_dispatch(
        &self,
        req: NatsRequest,
    ) -> Result<PreparedRequest, NatsErrorResponse> {
        Self::prepare_request_for_dispatch_with_state(&self.state, req)
    }

    pub(crate) fn prepare_request_for_dispatch_with_state(
        state: &StateMap,
        req: NatsRequest,
    ) -> Result<PreparedRequest, NatsErrorResponse> {
        prepare_request_for_dispatch_with_state(state, req)
    }

    async fn spawn_service(
        &self,
        svc_def: ServiceDefinition,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<LiveService> {
        let service_name = svc_def.metadata.name.clone();
        let service_version = svc_def.metadata.version.clone();
        let service_description = svc_def.metadata.description.clone();
        let mut workers = Vec::new();

        let service = self
            .client
            .service_builder()
            .description(service_description)
            .start(service_name.clone(), service_version.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        info!(
            service = %service_name,
            version = %service_version,
            "nats-micro service started"
        );

        workers.extend(
            self.spawn_consumers_for(&service_name, &svc_def.consumers, shutdown_rx.clone())
                .await?,
        );

        for endpoint_def in svc_def.endpoints {
            let app = self.clone();
            let endpoint_service_name = service_name.clone();
            let endpoint_group = endpoint_def.group.clone();
            let endpoint_subject = endpoint_def.subject.clone();
            let endpoint_full_subject = endpoint_def.full_subject();
            let queue_group = endpoint_def.queue_group.clone();
            let concurrency_limit =
                resolve_endpoint_concurrency_limit(endpoint_def.concurrency_limit);

            debug!(
                service = %endpoint_service_name,
                group = %endpoint_group,
                subject = %endpoint_subject,
                full_subject = %endpoint_full_subject,
                queue_group = ?queue_group,
                auth_required = endpoint_def.auth_required,
                requested_concurrency_limit = ?endpoint_def.concurrency_limit,
                concurrency_limit = concurrency_limit,
                "registering endpoint"
            );

            let mut ep = service
                .endpoint_builder()
                .name(endpoint_def.fn_name.clone())
                .add(endpoint_def.full_subject())
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            info!(
                service = %endpoint_service_name,
                group = %endpoint_group,
                subject = %endpoint_subject,
                full_subject = %endpoint_full_subject,
                concurrency_limit = concurrency_limit,
                "endpoint registered"
            );

            workers.push(tokio::spawn(run_endpoint_worker(
                app,
                endpoint_def,
                endpoint_service_name,
                ep,
                shutdown_rx.clone(),
            )));
        }

        Ok(LiveService {
            _service: service,
            workers,
        })
    }

    async fn spawn_consumers_for(
        &self,
        service_name: &str,
        consumers: &[ConsumerDefinition],
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Vec<JoinHandle<()>>> {
        if consumers.is_empty() {
            debug!(service = %service_name, "service has no consumers to spawn");
            return Ok(Vec::new());
        }

        let jetstream = jetstream::new(self.client.clone());
        let mut workers = Vec::new();

        for consumer_def in consumers {
            debug!(
                service = %service_name,
                stream = %consumer_def.stream,
                durable = %consumer_def.durable,
                auth_required = consumer_def.auth_required,
                requested_concurrency_limit = ?consumer_def.concurrency_limit,
                "initializing consumer"
            );

            let stream = jetstream
                .get_stream(consumer_def.stream.clone())
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to get JetStream stream `{}`: {e}",
                        consumer_def.stream
                    )
                })?;

            let durable = consumer_def.durable.clone();

            let deliver_subject = self.client.new_inbox();
            let mut consumer_config = consumer_def.config.clone();
            consumer_config.deliver_subject = deliver_subject;
            consumer_config.durable_name = Some(durable.clone());

            if let Some(limit) = consumer_def.concurrency_limit {
                validate_consumer_concurrency_limit(limit, consumer_config.max_ack_pending)
                    .map_err(|err| {
                        anyhow::anyhow!(
                            "consumer `{}` on stream `{}` has invalid configured concurrency limit: {err}",
                            durable,
                            consumer_def.stream
                        )
                    })?;
            }

            let consumer = stream
                .get_or_create_consumer(&durable, consumer_config)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to create JetStream consumer `{}` on stream `{}`: {e}",
                        durable,
                        consumer_def.stream
                    )
                })?;

            let cached_max_ack_pending = consumer.cached_info().config.max_ack_pending;
            if let Some(limit) = consumer_def.concurrency_limit {
                validate_consumer_concurrency_limit(limit, cached_max_ack_pending).map_err(
                    |err| {
                        anyhow::anyhow!(
                            "consumer `{}` on stream `{}` has invalid server concurrency limit: {err}",
                            durable,
                            consumer_def.stream
                        )
                    },
                )?;
            }

            let resolved_limit = resolve_consumer_concurrency_limit(
                consumer_def.concurrency_limit,
                cached_max_ack_pending,
            );
            if resolved_limit.promoted_from_default {
                info!(
                    service = %service_name,
                    stream = %consumer_def.stream,
                    durable = %durable,
                    default_concurrency_limit = DEFAULT_CONCURRENCY_LIMIT,
                    max_ack_pending = cached_max_ack_pending,
                    concurrency_limit = resolved_limit.value,
                    "consumer concurrency limit raised to match max_ack_pending"
                );
            } else {
                debug!(
                    service = %service_name,
                    stream = %consumer_def.stream,
                    durable = %durable,
                    max_ack_pending = cached_max_ack_pending,
                    concurrency_limit = resolved_limit.value,
                    "consumer concurrency limit resolved"
                );
            }

            let mut messages = consumer.messages().await.map_err(|e| {
                anyhow::anyhow!(
                    "failed to subscribe JetStream consumer `{}` messages: {e}",
                    durable
                )
            })?;

            let app = self.clone();
            let consumer_def = consumer_def.clone();
            let service_name = service_name.to_string();
            workers.push(tokio::spawn(run_consumer_worker(
                app,
                consumer_def,
                service_name,
                durable,
                messages,
                resolved_limit.value,
                shutdown_rx.clone(),
            )));
        }

        Ok(workers)
    }

    async fn resolve_user(
        &self,
        req: &NatsRequest,
        auth_required: bool,
    ) -> Result<Option<crate::auth::BoxAuthUser>, crate::auth::AuthError> {
        let Some(auth) = &self.auth else {
            return Ok(None);
        };

        let should_auth = auth_required || has_auth_headers(&req.headers);

        if !should_auth {
            return Ok(None);
        }

        auth.resolve(req).await.map(Some)
    }
}
