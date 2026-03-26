use anyhow::Result;
use async_nats::jetstream::{self, AckKind, consumer::push};
use async_nats::service::ServiceExt;
use futures::StreamExt;
use tracing::{error, info};

use crate::{
    auth::AuthConfig,
    consumer::ConsumerDefinition,
    error::IntoNatsError,
    handler::RequestContext,
    registry::{registered_consumers, registered_endpoints, registered_services},
    request::NatsRequest,
    service::{EndpointDefinition, ServiceMetadata},
    state::StateMap,
    utils::{ensure_request_id, has_auth_headers},
};

#[derive(Clone)]
pub struct NatsApp {
    client: async_nats::Client,
    state: StateMap,
    auth: Option<AuthConfig>,
    fallback_name: String,
    fallback_version: String,
    fallback_description: String,
    endpoints: Vec<EndpointDefinition>,
    consumers: Vec<ConsumerDefinition>,
    service_meta: Option<ServiceMetadata>,
}

impl NatsApp {
    pub fn new(client: async_nats::Client) -> Self {
        Self {
            client,
            state: StateMap::new(),
            auth: None,
            fallback_name: "app".to_string(),
            fallback_version: "0.1.0".to_string(),
            fallback_description: String::new(),
            endpoints: Vec::new(),
            consumers: Vec::new(),
            service_meta: None,
        }
    }

    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.fallback_name = name.into();
        self
    }

    pub fn service_version(mut self, version: impl Into<String>) -> Self {
        self.fallback_version = version.into();
        self
    }

    pub fn service_description(mut self, description: impl Into<String>) -> Self {
        self.fallback_description = description.into();
        self
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

    pub fn endpoint(mut self, def: EndpointDefinition) -> Self {
        self.endpoints.push(def);
        self
    }

    pub fn consumer_def(mut self, def: ConsumerDefinition) -> Self {
        self.consumers.push(def);
        self
    }

    pub fn consumer(self, def: ConsumerDefinition) -> Self {
        self.consumer_def(def)
    }

    pub fn service_metadata(mut self, meta: ServiceMetadata) -> Self {
        self.service_meta = Some(meta);
        self
    }

    pub async fn run(mut self) -> Result<()> {
        let registry_services = registered_services();
        let registry_endpoints = registered_endpoints();
        let registry_consumers = registered_consumers();

        self.endpoints.extend(registry_endpoints);
        self.consumers.extend(registry_consumers);

        let service_meta = self
            .service_meta
            .take()
            .or_else(|| registry_services.into_iter().next())
            .unwrap_or_else(|| {
                ServiceMetadata::new(
                    self.fallback_name.clone(),
                    self.fallback_version.clone(),
                    self.fallback_description.clone(),
                )
            });

        let service = self
            .client
            .service_builder()
            .description(service_meta.description.clone())
            .start(service_meta.name.clone(), service_meta.version.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        info!(
            service = %service_meta.name,
            version = %service_meta.version,
            "nats-micro service started"
        );

        self.spawn_consumers().await?;

        let endpoints = std::mem::take(&mut self.endpoints);

        for endpoint_def in endpoints {
            let app = self.clone();

            let group = service.group(endpoint_def.group.clone());
            let mut ep = group
                .endpoint(endpoint_def.subject.clone())
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            tokio::spawn(async move {
                while let Some(raw_req) = ep.next().await {
                    let headers = raw_req.message.headers.clone().unwrap_or_default();
                    let request_id = ensure_request_id(&headers);

                    let req = NatsRequest {
                        subject: raw_req.message.subject.to_string(),
                        payload: raw_req.message.payload.clone(),
                        headers,
                        reply: raw_req.message.reply.as_ref().map(|s| s.to_string()),
                        request_id: request_id.clone(),
                    };

                    let user = match app.resolve_user(&req, endpoint_def.auth_required).await {
                        Ok(user) => user,
                        Err(err) => {
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
                            let payload = serde_json::to_vec(&err).unwrap_or_default();
                            let _ = raw_req.respond(Ok(payload.into())).await;
                            continue;
                        }
                    };

                    if let Err(err) = raw_req.respond(Ok(response.payload)).await {
                        error!(error = %err, request_id = %request_id, "failed to respond");
                    }
                }
            });
        }

        tokio::signal::ctrl_c().await?;
        Ok(())
    }

    async fn spawn_consumers(&self) -> Result<()> {
        let consumers = self.consumers.clone();

        if consumers.is_empty() {
            return Ok(());
        }

        let jetstream = jetstream::new(self.client.clone());

        for consumer_def in consumers {
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
            tokio::spawn(async move {
                while let Some(message) = messages.next().await {
                    let message = match message {
                        Ok(message) => message,
                        Err(err) => {
                            error!(error = %err, consumer = %durable, "failed to receive consumer message");
                            continue;
                        }
                    };

                    let headers = message.headers.clone().unwrap_or_default();
                    let request_id = ensure_request_id(&headers);
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
                            if let Err(err) = message.ack().await {
                                error!(
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
                                error = %err,
                                consumer = %durable,
                                request_id = %request_id,
                                "consumer handler failed"
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
