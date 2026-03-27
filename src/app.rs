use anyhow::Result;
use async_nats::jetstream::{self, AckKind, consumer::push};
use async_nats::service::ServiceExt;
use futures::StreamExt;
use tracing::{debug, error, info};

use crate::{
    auth::AuthConfig,
    consumer::ConsumerDefinition,
    error::IntoNatsError,
    handler::RequestContext,
    request::NatsRequest,
    service::NatsService,
    service::ServiceDefinition,
    state::StateMap,
    utils::{ensure_request_id, has_auth_headers},
};

#[derive(Clone)]
pub struct NatsApp {
    client: async_nats::Client,
    state: StateMap,
    auth: Option<AuthConfig>,
    service_defs: Vec<ServiceDefinition>,
}

impl NatsApp {
    pub fn new(client: async_nats::Client) -> Self {
        Self {
            client,
            state: StateMap::new(),
            auth: None,
            service_defs: Vec::new(),
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

    pub fn service_def(mut self, def: ServiceDefinition) -> Self {
        self.service_defs.push(def);
        self
    }

    pub async fn run(mut self) -> Result<()> {
        if self.service_defs.is_empty() {
            anyhow::bail!("NatsApp requires at least one explicit service via .service(...) or .service_def(...)");
        }

        let mut live_services = Vec::new();
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
            let svc = self.spawn_service(svc_def).await?;
            live_services.push(svc);
        }

        info!(service_count = live_services.len(), "all services are running");
        tokio::signal::ctrl_c().await?;
        drop(live_services);
        Ok(())
    }

    async fn spawn_service(
        &self,
        svc_def: ServiceDefinition,
    ) -> Result<async_nats::service::Service> {
        let service_name = svc_def.metadata.name.clone();
        let service_version = svc_def.metadata.version.clone();
        let service_description = svc_def.metadata.description.clone();

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

        self.spawn_consumers_for(&service_name, &svc_def.consumers).await?;

        for endpoint_def in svc_def.endpoints {
            let app = self.clone();
            let endpoint_service_name = service_name.clone();
            let endpoint_group = endpoint_def.group.clone();
            let endpoint_subject = endpoint_def.subject.clone();
            let endpoint_full_subject = endpoint_def.full_subject();
            let queue_group = endpoint_def.queue_group.clone();

            debug!(
                service = %endpoint_service_name,
                group = %endpoint_group,
                subject = %endpoint_subject,
                full_subject = %endpoint_full_subject,
                queue_group = ?queue_group,
                auth_required = endpoint_def.auth_required,
                "registering endpoint"
            );

            let mut ep = if endpoint_group.is_empty() {
                service
                    .endpoint(endpoint_subject.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?
            } else {
                service
                    .group(endpoint_group.clone())
                    .endpoint(endpoint_subject.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?
            };

            info!(
                service = %endpoint_service_name,
                group = %endpoint_group,
                subject = %endpoint_subject,
                full_subject = %endpoint_full_subject,
                "endpoint registered"
            );

            tokio::spawn(async move {
                while let Some(raw_req) = ep.next().await {
                    let headers = raw_req.message.headers.clone().unwrap_or_default();
                    let request_id = ensure_request_id(&headers);

                    debug!(
                        service = %endpoint_service_name,
                        group = %endpoint_group,
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
                    let request_subject = req.subject.clone();

                    let user = match app.resolve_user(&req, endpoint_def.auth_required).await {
                        Ok(user) => user,
                        Err(err) => {
                            debug!(
                                service = %endpoint_service_name,
                                group = %endpoint_group,
                                subject = %request_subject,
                                request_id = %request_id,
                                "request authentication failed"
                            );
                            let err = err.into_nats_error(request_id.clone());
                            let payload = serde_json::to_vec(&err).unwrap_or_default();
                            let _ = raw_req.respond(Ok(payload.into())).await;
                            continue;
                        }
                    };

                    let ctx = RequestContext {
                        request: req,
                        states: app.state.clone(),
                        user,
                        subject_template: endpoint_def.subject_template.clone(),
                        current_param_name: None,
                    };

                    let response = match endpoint_def.handler.call(ctx).await {
                        Ok(res) => res,
                        Err(err) => {
                            error!(
                                service = %endpoint_service_name,
                                group = %endpoint_group,
                                subject = %request_subject,
                                request_id = %request_id,
                                error = %err,
                                "endpoint handler failed"
                            );
                            let payload = serde_json::to_vec(&err).unwrap_or_default();
                            let _ = raw_req.respond(Ok(payload.into())).await;
                            continue;
                        }
                    };

                    debug!(
                        service = %endpoint_service_name,
                        group = %endpoint_group,
                        subject = %request_subject,
                        request_id = %request_id,
                        "sending endpoint response"
                    );

                    if let Err(err) = raw_req.respond(Ok(response.payload)).await {
                        error!(
                            service = %endpoint_service_name,
                            group = %endpoint_group,
                            subject = %request_subject,
                            request_id = %request_id,
                            error = %err,
                            "failed to respond"
                        );
                    }
                }
            });
        }

        Ok(service)
    }

    async fn spawn_consumers_for(
        &self,
        service_name: &str,
        consumers: &[ConsumerDefinition],
    ) -> Result<()> {
        if consumers.is_empty() {
            debug!(service = %service_name, "service has no consumers to spawn");
            return Ok(());
        }

        let jetstream = jetstream::new(self.client.clone());

        for consumer_def in consumers {
            debug!(
                service = %service_name,
                stream = %consumer_def.stream,
                durable = %consumer_def.durable,
                filter_subject = %consumer_def.filter_subject,
                ack_on_success = consumer_def.ack_on_success,
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

            let deliver_subject = self.client.new_inbox();
            let durable = consumer_def.durable.clone();
            let consumer = stream
                .get_or_create_consumer(
                    &durable,
                    push::Config {
                        durable_name: Some(durable.clone()),
                        deliver_subject,
                        filter_subject: consumer_def.filter_subject.clone(),
                        ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to create JetStream consumer `{}` on stream `{}`: {e}",
                        durable,
                        consumer_def.stream
                    )
                })?;

            let mut messages = consumer.messages().await.map_err(|e| {
                anyhow::anyhow!(
                    "failed to subscribe JetStream consumer `{}` messages: {e}",
                    durable
                )
            })?;

            let app = self.clone();
            let consumer_def = consumer_def.clone();
            let service_name = service_name.to_string();
            tokio::spawn(async move {
                while let Some(message) = messages.next().await {
                    let message = match message {
                        Ok(message) => message,
                        Err(err) => {
                            error!(
                                service = %service_name,
                                consumer = %durable,
                                error = %err,
                                "failed to receive consumer message"
                            );
                            continue;
                        }
                    };

                    let headers = message.headers.clone().unwrap_or_default();
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

                    let user = match app.resolve_user(&req, false).await {
                        Ok(user) => user,
                        Err(err) => {
                            error!(
                                service = %service_name,
                                error = %err,
                                consumer = %durable,
                                request_id = %request_id,
                                "consumer auth resolution failed"
                            );
                            let _ = message.ack_with(AckKind::Nak(None)).await;
                            continue;
                        }
                    };

                    let ctx = RequestContext {
                        request: req,
                        states: app.state.clone(),
                        user,
                        subject_template: None,
                        current_param_name: None,
                    };

                    match consumer_def.handler.call(ctx).await {
                        Ok(_) if consumer_def.ack_on_success => {
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
                        Ok(_) => {}
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
            });
        }

        Ok(())
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
