#![allow(
    clippy::assigning_clones,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::unused_async,
    clippy::used_underscore_binding
)]

use std::{
    collections::BTreeSet,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use futures::StreamExt;
#[cfg(feature = "napi")]
use nats_micro::napi;
#[cfg(feature = "client")]
use nats_micro::{ClientCallOptions, Json, NatsService, Proto, SubjectParam};
use nats_micro::{
    ConsumerDefinition, EndpointDefinition, NatsApp, NatsErrorResponse, Payload, ServiceDefinition,
    ServiceMetadata, ShutdownSignal, State, WorkerFailurePolicy, async_nats, service,
    service_error, service_handlers,
};
#[cfg(all(feature = "client", feature = "encryption"))]
use nats_micro::{Encrypted, ServiceKeyPair};
#[cfg(feature = "client")]
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::timeout,
};
use uuid::Uuid;

#[service(name = "live-queue-alpha")]
struct LiveQueueAlphaService;

#[service_handlers]
impl LiveQueueAlphaService {
    #[endpoint(subject = "jobs", group = "live", queue_group = "alpha-live-workers")]
    async fn jobs() -> Result<&'static str, NatsErrorResponse> {
        Ok("alpha")
    }
}

#[service(name = "live-queue-beta")]
struct LiveQueueBetaService;

#[service_handlers]
impl LiveQueueBetaService {
    #[endpoint(subject = "jobs", group = "live", queue_group = "beta-live-workers")]
    async fn jobs() -> Result<&'static str, NatsErrorResponse> {
        Ok("beta")
    }
}

#[service(name = "live-supervision")]
struct LiveSupervisionService;

#[service_handlers]
impl LiveSupervisionService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }
}

#[cfg(feature = "client")]
#[cfg_attr(feature = "napi", nats_micro::object)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveClientSumRequest {
    pub numbers: Vec<i64>,
}

#[cfg(feature = "client")]
#[cfg_attr(feature = "napi", nats_micro::object)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveClientSumResponse {
    pub total: i64,
}

#[cfg(feature = "client")]
#[cfg_attr(feature = "napi", nats_micro::object)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveOptionalJsonPayload {
    pub value: String,
}

#[cfg(feature = "client")]
#[cfg_attr(feature = "napi", nats_micro::object)]
#[derive(Clone, PartialEq, nats_micro::prost::Message)]
pub struct LiveOptionalProtoPayload {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[cfg(feature = "client")]
#[service_error]
#[derive(Debug, thiserror::Error)]
enum LiveGeneratedClientError {
    #[error("consumer event kind was empty")]
    InvalidEvent,
}

#[cfg(feature = "client")]
#[cfg_attr(feature = "napi", service(name = "live-generated-client", napi = true))]
#[cfg_attr(not(feature = "napi"), service(name = "live-generated-client"))]
struct LiveGeneratedClientService;

#[cfg(feature = "client")]
#[service_handlers]
impl LiveGeneratedClientService {
    #[endpoint(subject = "health", group = "live-client")]
    async fn health() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[endpoint(subject = "sum", group = "live-client")]
    async fn sum(
        payload: Payload<Json<LiveClientSumRequest>>,
    ) -> Result<Json<LiveClientSumResponse>, NatsErrorResponse> {
        Ok(Json(LiveClientSumResponse {
            total: payload.numbers.iter().sum(),
        }))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "live-client")]
    async fn get_user_profile(user_id: SubjectParam<String>) -> Result<String, NatsErrorResponse> {
        Ok(format!("profile:{}", user_id.as_str()))
    }

    #[endpoint(subject = "maybe-json", group = "live-client")]
    async fn maybe_json(
        payload: Payload<Option<Json<LiveOptionalJsonPayload>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .as_deref()
            .map(|payload| payload.value.clone())
            .unwrap_or_else(|| "none".to_string()))
    }

    #[endpoint(subject = "maybe-proto", group = "live-client")]
    async fn maybe_proto(
        payload: Payload<Option<Proto<LiveOptionalProtoPayload>>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .as_deref()
            .map(|payload| payload.value.clone())
            .unwrap_or_else(|| "none".to_string()))
    }

    #[endpoint(subject = "maybe-string-response", group = "live-client")]
    async fn maybe_string_response(
        payload: Payload<Option<String>>,
    ) -> Result<Option<String>, NatsErrorResponse> {
        Ok(payload.into_inner())
    }

    #[endpoint(subject = "maybe-json-response", group = "live-client")]
    async fn maybe_json_response(
        payload: Payload<Option<Json<LiveOptionalJsonPayload>>>,
    ) -> Result<Option<Json<LiveOptionalJsonPayload>>, NatsErrorResponse> {
        Ok(payload.into_inner())
    }

    #[endpoint(subject = "maybe-proto-response", group = "live-client")]
    async fn maybe_proto_response(
        payload: Payload<Option<Proto<LiveOptionalProtoPayload>>>,
    ) -> Result<Option<Proto<LiveOptionalProtoPayload>>, NatsErrorResponse> {
        Ok(payload.into_inner())
    }

    #[endpoint(subject = "service-error", group = "live-client")]
    async fn service_error() -> Result<(), LiveGeneratedClientError> {
        Err(LiveGeneratedClientError::InvalidEvent)
    }

    #[cfg(feature = "encryption")]
    #[endpoint(subject = "maybe-encrypted", group = "live-client")]
    async fn maybe_encrypted(
        payload: Payload<Option<Encrypted<String>>>,
        _shutdown: ShutdownSignal,
    ) -> Result<String, NatsErrorResponse> {
        Ok(payload
            .as_deref()
            .cloned()
            .unwrap_or_else(|| "none".to_string()))
    }

    #[cfg(feature = "encryption")]
    #[endpoint(subject = "maybe-encrypted-response", group = "live-client")]
    async fn maybe_encrypted_response(
        payload: Payload<Option<Encrypted<String>>>,
        _shutdown: ShutdownSignal,
    ) -> Result<Option<Encrypted<String>>, NatsErrorResponse> {
        Ok(payload.into_inner())
    }
}

#[derive(Clone, Default)]
struct ConsumerProbe {
    hits: Arc<AtomicUsize>,
    processed: Arc<Notify>,
}

impl ConsumerProbe {
    fn record(&self) {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.processed.notify_waiters();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShutdownObservation {
    requested: bool,
    drain_timeout: Option<Duration>,
    deadline_present: bool,
}

#[derive(Clone, Default)]
struct ShutdownProbe {
    started: Arc<Notify>,
    observation: Arc<Mutex<Option<ShutdownObservation>>>,
}

impl ShutdownProbe {
    fn mark_started(&self) {
        self.started.notify_waiters();
    }

    fn record(&self, shutdown: &ShutdownSignal) {
        *self.observation.lock().unwrap() = Some(ShutdownObservation {
            requested: shutdown.is_requested(),
            drain_timeout: shutdown.drain_timeout(),
            deadline_present: shutdown.deadline().is_some(),
        });
    }

    fn observation(&self) -> ShutdownObservation {
        self.observation
            .lock()
            .unwrap()
            .clone()
            .expect("shutdown observation should be recorded")
    }
}

#[service(name = "live-consumer-flow")]
struct LiveConsumerFlowService;

#[service_handlers]
impl LiveConsumerFlowService {
    #[consumer(
        stream = "LIVE_STREAM",
        durable = "LIVE_DURABLE",
        config = nats_micro::ConsumerConfig {
            ack_wait: std::time::Duration::from_secs(1),
            ..Default::default()
        }
    )]
    async fn jobs(
        _payload: Payload<String>,
        probe: State<ConsumerProbe>,
    ) -> Result<(), NatsErrorResponse> {
        probe.record();
        Ok(())
    }
}

#[service(name = "live-shutdown-endpoint")]
struct LiveShutdownEndpointService;

#[service_handlers]
impl LiveShutdownEndpointService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[endpoint(subject = "cleanup", group = "live")]
    async fn cleanup(
        mut shutdown: ShutdownSignal,
        probe: State<ShutdownProbe>,
    ) -> Result<&'static str, NatsErrorResponse> {
        probe.mark_started();
        shutdown.wait_for_shutdown().await;
        probe.record(&shutdown);
        Ok("cleaned")
    }
}

#[service(name = "live-shutdown-consumer")]
struct LiveShutdownConsumerService;

#[service_handlers]
impl LiveShutdownConsumerService {
    #[endpoint(subject = "status", group = "live")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[consumer(
        stream = "LIVE_SHUTDOWN_STREAM",
        durable = "LIVE_SHUTDOWN_DURABLE",
        config = nats_micro::ConsumerConfig {
            ack_wait: std::time::Duration::from_secs(1),
            ..Default::default()
        }
    )]
    async fn jobs(
        _payload: Payload<String>,
        mut shutdown: ShutdownSignal,
        probe: State<ShutdownProbe>,
    ) -> Result<(), NatsErrorResponse> {
        probe.mark_started();
        shutdown.wait_for_shutdown().await;
        probe.record(&shutdown);
        Ok(())
    }
}

#[test]
fn macro_generated_handlers_only_enable_shutdown_support_when_needed() {
    assert!(
        !LiveQueueAlphaService::jobs_endpoint()
            .handler
            .requires_shutdown_signal()
    );
    assert!(
        !LiveConsumerFlowService::jobs_consumer()
            .handler
            .requires_shutdown_signal()
    );
    assert!(
        LiveShutdownEndpointService::cleanup_endpoint()
            .handler
            .requires_shutdown_signal()
    );
    assert!(
        LiveShutdownConsumerService::jobs_consumer()
            .handler
            .requires_shutdown_signal()
    );
}

#[tokio::test]
async fn live_queue_groups_receive_independent_copies() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let unique = unique_name("queue-groups");
    let group = format!("live.{unique}");
    let full_subject = format!("{group}.jobs");

    let mut alpha_endpoint = LiveQueueAlphaService::jobs_endpoint();
    alpha_endpoint.group = group.clone();

    let mut beta_endpoint = LiveQueueBetaService::jobs_endpoint();
    beta_endpoint.group = group.clone();

    let (alpha_shutdown, alpha_handle) =
        spawn_app_until_shutdown(NatsApp::new(client.clone()).service_def(endpoint_service(
            unique_name("alpha-service"),
            alpha_endpoint,
        )));
    let (beta_shutdown, beta_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .service_def(endpoint_service(unique_name("beta-service"), beta_endpoint)),
    );

    let replies_result = request_expected_replies(&client, &full_subject, 2).await;

    let _ = alpha_shutdown.send(());
    let _ = beta_shutdown.send(());

    let alpha_result = alpha_handle.await.context("alpha app task failed")?;
    let beta_result = beta_handle.await.context("beta app task failed")?;

    let replies = replies_result?;
    let actual: BTreeSet<_> = replies.into_iter().collect();
    let expected: BTreeSet<_> = ["alpha".to_string(), "beta".to_string()]
        .into_iter()
        .collect();

    assert_eq!(actual, expected);
    alpha_result?;
    beta_result?;
    Ok(())
}

#[tokio::test]
async fn live_client_drain_triggers_supervised_shutdown() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let unique = unique_name("supervision");
    let group = format!("live.{unique}");
    let full_subject = format!("{group}.status");

    let mut endpoint = LiveSupervisionService::status_endpoint();
    endpoint.group = group;

    let app_handle = tokio::spawn(
        NatsApp::new(client.clone())
            .with_worker_failure_policy(WorkerFailurePolicy::ShutdownApp)
            .service_def(endpoint_service(
                unique_name("supervision-service"),
                endpoint,
            ))
            .run_until(async {
                pending::<()>().await;
                Ok(())
            }),
    );

    let readiness = request_expected_replies(&client, &full_subject, 1).await;
    client
        .drain()
        .await
        .context("failed to drain the shared client")?;
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for supervised shutdown")?
        .context("supervision app task failed")?;

    readiness?;
    let err = app_result.expect_err("client drain should trigger supervised shutdown");
    assert!(err.to_string().contains("worker `endpoint"));
    Ok(())
}

#[cfg(feature = "client")]
#[tokio::test]
async fn live_generated_client_round_trips_standard_endpoints() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let (shutdown_tx, app_handle, service_client) =
        spawn_live_generated_client_service(client.clone()).await?;

    let standard_result = async {
        let health = service_client
            .health()
            .await
            .context("generated client health call failed")?;
        assert_eq!(health, "ok");

        let health_with = service_client
            .health_with(ClientCallOptions::new().header("x-trace-id", "health-1"))
            .await
            .context("generated client health_with call failed")?;
        assert_eq!(health_with, "ok");

        let sum = service_client
            .sum(&LiveClientSumRequest {
                numbers: vec![1, 2, 3, 4],
            })
            .await
            .context("generated client sum call failed")?;
        assert_eq!(sum, LiveClientSumResponse { total: 10 });

        let sum_with = service_client
            .sum_with(
                &LiveClientSumRequest {
                    numbers: vec![9, 1],
                },
                ClientCallOptions::new().header("x-trace-id", "sum-1"),
            )
            .await
            .context("generated client sum_with call failed")?;
        assert_eq!(sum_with.total, 10);

        let profile = service_client
            .get_user_profile(&"alice".to_string())
            .await
            .context("generated client subject-param call failed")?;
        assert_eq!(profile, "profile:alice");

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result = shutdown_app(shutdown_tx, app_handle, "generated client standard").await;

    standard_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(feature = "client")]
#[tokio::test]
async fn live_generated_client_optional_payload_variants_round_trip() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let (shutdown_tx, app_handle, service_client) =
        spawn_live_generated_client_service(client.clone()).await?;

    let optional_result = async {
        let json_some = service_client
            .maybe_json(Some(&LiveOptionalJsonPayload {
                value: "json-value".to_string(),
            }))
            .await
            .context("generated client optional json Some call failed")?;
        assert_eq!(json_some, "json-value");

        let json_none = service_client
            .maybe_json(None)
            .await
            .context("generated client optional json None call failed")?;
        assert_eq!(json_none, "none");

        let proto_some = service_client
            .maybe_proto(Some(&LiveOptionalProtoPayload {
                value: "proto-value".to_string(),
            }))
            .await
            .context("generated client optional proto Some call failed")?;
        assert_eq!(proto_some, "proto-value");

        let proto_none = service_client
            .maybe_proto(None)
            .await
            .context("generated client optional proto None call failed")?;
        assert_eq!(proto_none, "none");

        #[cfg(feature = "encryption")]
        {
            let encrypted_some = service_client
                .maybe_encrypted(Some("secret-value"))
                .await
                .context("generated client optional encrypted Some call failed")?;
            assert_eq!(encrypted_some, "secret-value");

            let encrypted_none = service_client
                .maybe_encrypted(None)
                .await
                .context("generated client optional encrypted None call failed")?;
            assert_eq!(encrypted_none, "none");
        }

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result = shutdown_app(shutdown_tx, app_handle, "generated client optional").await;

    optional_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(feature = "client")]
#[tokio::test]
async fn live_generated_client_optional_response_variants_round_trip() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let (shutdown_tx, app_handle, service_client) =
        spawn_live_generated_client_service(client.clone()).await?;

    let optional_result = async {
        let string_some = service_client
            .maybe_string_response(Some("plain-value"))
            .await
            .context("generated client optional string response Some call failed")?;
        assert_eq!(string_some, Some("plain-value".to_string()));

        let string_none = service_client
            .maybe_string_response(None)
            .await
            .context("generated client optional string response None call failed")?;
        assert_eq!(string_none, None);

        let json_some = service_client
            .maybe_json_response(Some(&LiveOptionalJsonPayload {
                value: "json-response".to_string(),
            }))
            .await
            .context("generated client optional json response Some call failed")?;
        assert_eq!(
            json_some,
            Some(LiveOptionalJsonPayload {
                value: "json-response".to_string(),
            })
        );

        let json_none = service_client
            .maybe_json_response(None)
            .await
            .context("generated client optional json response None call failed")?;
        assert_eq!(json_none, None);

        let proto_some = service_client
            .maybe_proto_response(Some(&LiveOptionalProtoPayload {
                value: "proto-response".to_string(),
            }))
            .await
            .context("generated client optional proto response Some call failed")?;
        assert_eq!(
            proto_some,
            Some(LiveOptionalProtoPayload {
                value: "proto-response".to_string(),
            })
        );

        let proto_none = service_client
            .maybe_proto_response(None)
            .await
            .context("generated client optional proto response None call failed")?;
        assert_eq!(proto_none, None);

        #[cfg(feature = "encryption")]
        {
            let encrypted_some = service_client
                .maybe_encrypted_response(Some("secret-response"))
                .await
                .context("generated client optional encrypted response Some call failed")?;
            assert_eq!(encrypted_some, Some("secret-response".to_string()));

            let encrypted_none = service_client
                .maybe_encrypted_response(None)
                .await
                .context("generated client optional encrypted response None call failed")?;
            assert_eq!(encrypted_none, None);
        }

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result = shutdown_app(
        shutdown_tx,
        app_handle,
        "generated client optional response",
    )
    .await;

    optional_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(feature = "client")]
#[tokio::test]
async fn live_generated_client_preserves_service_error_metadata() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let (shutdown_tx, app_handle, service_client) =
        spawn_live_generated_client_service(client.clone()).await?;

    let service_result = async {
        let error = service_client
            .service_error()
            .await
            .expect_err("generated client service_error call should fail");
        let response = error.into_nats_error_response();

        assert_eq!(response.kind, "INVALID_EVENT");
        assert_eq!(response.message, "consumer event kind was empty");
        assert!(!response.request_id.is_empty());

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result =
        shutdown_app(shutdown_tx, app_handle, "generated client service error").await;

    service_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(all(feature = "client", feature = "napi"))]
#[tokio::test]
async fn live_generated_napi_client_round_trips_endpoints() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let server = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let spawned = spawn_live_generated_client_service_app(client).await?;

    let napi_result = async {
        let service_client = build_live_generated_napi_client(
            server,
            spawned.prefix.clone(),
            #[cfg(feature = "encryption")]
            spawned.recipient,
        )
        .await
        .map_err(|err| anyhow::anyhow!("generated N-API client connect failed: {err}"))?;

        let health = service_client
            .health()
            .await
            .map_err(|err| anyhow::anyhow!("generated N-API client health call failed: {err}"))?;
        assert_eq!(health, "ok");

        let sum = service_client
            .sum(LiveClientSumRequest {
                numbers: vec![3, 4, 5],
            })
            .await
            .map_err(|err| anyhow::anyhow!("generated N-API client sum call failed: {err}"))?;
        assert_eq!(sum, LiveClientSumResponse { total: 12 });

        let profile = service_client
            .get_user_profile(LiveGeneratedClientServiceGetUserProfileArgs {
                user_id: "alice".to_string(),
            })
            .await
            .map_err(|err| {
                anyhow::anyhow!("generated N-API client subject-param call failed: {err}")
            })?;
        assert_eq!(profile, "profile:alice");

        let json_some = service_client
            .maybe_json(Some(LiveOptionalJsonPayload {
                value: "json-value".to_string(),
            }))
            .await
            .map_err(|err| {
                anyhow::anyhow!("generated N-API client optional json Some call failed: {err}")
            })?;
        assert_eq!(json_some, "json-value");

        let json_none = service_client.maybe_json(None).await.map_err(|err| {
            anyhow::anyhow!("generated N-API client optional json None call failed: {err}")
        })?;
        assert_eq!(json_none, "none");

        let proto_some = service_client
            .maybe_proto(Some(LiveOptionalProtoPayload {
                value: "proto-value".to_string(),
            }))
            .await
            .map_err(|err| {
                anyhow::anyhow!("generated N-API client optional proto Some call failed: {err}")
            })?;
        assert_eq!(proto_some, "proto-value");

        let proto_none = service_client.maybe_proto(None).await.map_err(|err| {
            anyhow::anyhow!("generated N-API client optional proto None call failed: {err}")
        })?;
        assert_eq!(proto_none, "none");

        let string_some = service_client
            .maybe_string_response(Some("plain-value".to_string()))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "generated N-API client optional string response Some call failed: {err}"
                )
            })?;
        assert_eq!(string_some, Some("plain-value".to_string()));

        let string_none = service_client
            .maybe_string_response(None)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "generated N-API client optional string response None call failed: {err}"
                )
            })?;
        assert_eq!(string_none, None);

        let json_response_some = service_client
            .maybe_json_response(Some(LiveOptionalJsonPayload {
                value: "json-response".to_string(),
            }))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "generated N-API client optional json response Some call failed: {err}"
                )
            })?;
        assert_eq!(
            json_response_some,
            Some(LiveOptionalJsonPayload {
                value: "json-response".to_string(),
            })
        );

        let json_response_none = service_client
            .maybe_json_response(None)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "generated N-API client optional json response None call failed: {err}"
                )
            })?;
        assert_eq!(json_response_none, None);

        let proto_response_some = service_client
            .maybe_proto_response(Some(LiveOptionalProtoPayload {
                value: "proto-response".to_string(),
            }))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "generated N-API client optional proto response Some call failed: {err}"
                )
            })?;
        assert_eq!(
            proto_response_some,
            Some(LiveOptionalProtoPayload {
                value: "proto-response".to_string(),
            })
        );

        let proto_response_none =
            service_client
                .maybe_proto_response(None)
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "generated N-API client optional proto response None call failed: {err}"
                    )
                })?;
        assert_eq!(proto_response_none, None);

        let service_error = service_client
            .service_error()
            .await
            .expect_err("generated N-API client service_error call should fail");
        assert_eq!(service_error.status, "INVALID_EVENT");
        assert_eq!(service_error.reason, "consumer event kind was empty");

        assert_eq!(
            JsLiveGeneratedClientError::INVALID_EVENT.as_ref(),
            "INVALID_EVENT"
        );

        #[cfg(feature = "encryption")]
        {
            let encrypted_some = service_client
                .maybe_encrypted(Some("secret-value".to_string()))
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "generated N-API client optional encrypted Some call failed: {err}"
                    )
                })?;
            assert_eq!(encrypted_some, "secret-value");

            let encrypted_none = service_client.maybe_encrypted(None).await.map_err(|err| {
                anyhow::anyhow!("generated N-API client optional encrypted None call failed: {err}")
            })?;
            assert_eq!(encrypted_none, "none");

            let encrypted_response_some = service_client
                .maybe_encrypted_response(Some("secret-response".to_string()))
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "generated N-API client optional encrypted response Some call failed: {err}"
                    )
                })?;
            assert_eq!(encrypted_response_some, Some("secret-response".to_string()));

            let encrypted_response_none = service_client
                .maybe_encrypted_response(None)
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "generated N-API client optional encrypted response None call failed: {err}"
                    )
                })?;
            assert_eq!(encrypted_response_none, None);
        }

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result = shutdown_app(
        spawned.shutdown_tx,
        spawned.app_handle,
        "generated N-API client",
    )
    .await;

    napi_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(all(feature = "client", feature = "napi", feature = "encryption"))]
#[tokio::test]
async fn live_generated_napi_client_surfaces_framework_errors() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let server = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let spawned = spawn_live_generated_client_service_app(client).await?;

    let napi_result = async {
        let service_client =
            build_live_generated_napi_client_without_recipient(server, spawned.prefix.clone())
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "generated N-API client connect without recipient failed: {err}"
                    )
                })?;

        let error = service_client
            .maybe_encrypted(Some("secret-value".to_string()))
            .await
            .expect_err("generated N-API client encrypted call should fail without recipient");
        assert_eq!(error.status, "MISSING_RECIPIENT_PUBKEY");
        assert!(error.reason.contains("recipient"));

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let shutdown_result = shutdown_app(
        spawned.shutdown_tx,
        spawned.app_handle,
        "generated N-API framework error client",
    )
    .await;

    napi_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(all(feature = "client", feature = "napi"))]
#[tokio::test]
async fn live_generated_napi_client_connect_surfaces_auth_mode_conflict() -> Result<()> {
    let error = match JsLiveGeneratedClientServiceClient::connect(
        "nats://127.0.0.1:4222".to_string(),
        Some(LiveGeneratedClientServiceClientConnectOptions {
            auth: Some(LiveGeneratedClientServiceClientAuthOptions {
                token: Some("token".to_string()),
                username: Some("user".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    {
        Ok(_) => {
            anyhow::bail!("generated N-API client connect should reject conflicting auth modes")
        }
        Err(error) => error,
    };

    assert_eq!(error.status, "AUTH_MODE_CONFLICT");
    assert!(error.reason.contains("choose only one auth mode"));

    Ok(())
}

#[cfg(all(feature = "client", feature = "napi"))]
#[tokio::test]
async fn live_generated_napi_client_connect_surfaces_missing_auth_password() -> Result<()> {
    let error = match JsLiveGeneratedClientServiceClient::connect(
        "nats://127.0.0.1:4222".to_string(),
        Some(LiveGeneratedClientServiceClientConnectOptions {
            auth: Some(LiveGeneratedClientServiceClientAuthOptions {
                username: Some("user".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    {
        Ok(_) => anyhow::bail!(
            "generated N-API client connect should require a password with username auth"
        ),
        Err(error) => error,
    };

    assert_eq!(error.status, "AUTH_PASSWORD_REQUIRED");
    assert!(error.reason.contains("password is required"));

    Ok(())
}

#[cfg(all(feature = "client", feature = "napi", feature = "encryption"))]
#[tokio::test]
async fn live_generated_napi_client_connect_surfaces_invalid_recipient_public_key() -> Result<()> {
    let error = match JsLiveGeneratedClientServiceClient::connect(
        "nats://127.0.0.1:4222".to_string(),
        Some(LiveGeneratedClientServiceClientConnectOptions {
            recipient_public_key: Some(nats_micro::napi::bindgen_prelude::Buffer::from(vec![
                1, 2, 3,
            ])),
            ..Default::default()
        }),
    )
    .await
    {
        Ok(_) => anyhow::bail!(
            "generated N-API client connect should reject invalid recipient public keys"
        ),
        Err(error) => error,
    };

    assert_eq!(error.status, "MISSING_RECIPIENT_PUBKEY");
    assert!(error.reason.contains("exactly 32 bytes"));

    Ok(())
}

#[tokio::test]
async fn live_jetstream_consumer_processes_messages() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.events", unique_name("consumer-subject"));
    let durable = unique_name("durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live JetStream runtime test: {err}");
            return Ok(());
        }
    };

    let probe = ConsumerProbe::default();
    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable;
    consumer.config.filter_subject = subject.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let publish_result = async {
        jetstream
            .publish(subject.clone(), "hello".into())
            .await
            .context("failed to publish live JetStream test message")?
            .await
            .context("failed to await publish ack")?;

        timeout(Duration::from_secs(5), probe.processed.notified())
            .await
            .context("timed out waiting for live consumer to process message")?;

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let _ = shutdown_tx.send(());
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app shutdown")?
        .context("consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    publish_result?;
    app_result?;
    assert_eq!(probe.hits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn live_endpoint_handlers_observe_shutdown_signal_and_timeout() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let probe = ShutdownProbe::default();
    let unique = unique_name("shutdown-endpoint");
    let group = format!("live.{unique}");
    let status_subject = format!("{group}.status");
    let cleanup_subject = format!("{group}.cleanup");

    let mut status_endpoint = LiveShutdownEndpointService::status_endpoint();
    status_endpoint.group = group.clone();

    let mut cleanup_endpoint = LiveShutdownEndpointService::cleanup_endpoint();
    cleanup_endpoint.group = group.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .with_shutdown_drain_timeout(Duration::from_secs(2))
            .service_def(service_definition(
                unique_name("shutdown-endpoint-service"),
                vec![status_endpoint, cleanup_endpoint],
                Vec::new(),
            )),
    );

    request_expected_replies(&client, &status_subject, 1).await?;

    let request_client = client.clone();
    let request_handle = tokio::spawn(async move {
        let message = timeout(
            Duration::from_secs(5),
            request_client.request(cleanup_subject, "".into()),
        )
        .await
        .context("timed out waiting for cleanup endpoint response")?
        .context("cleanup endpoint request failed")?;

        String::from_utf8(message.payload.to_vec()).context("cleanup reply was not valid UTF-8")
    });

    timeout(Duration::from_secs(5), probe.started.notified())
        .await
        .context("timed out waiting for endpoint handler to start")?;

    let _ = shutdown_tx.send(());

    let reply = request_handle
        .await
        .context("cleanup request task failed")??;
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for endpoint app shutdown")?
        .context("endpoint app task failed")?;

    assert_eq!(reply, "cleaned");
    assert_eq!(
        probe.observation(),
        ShutdownObservation {
            requested: true,
            drain_timeout: Some(Duration::from_secs(2)),
            deadline_present: true,
        }
    );
    app_result?;
    Ok(())
}

#[tokio::test]
async fn live_consumer_handlers_observe_shutdown_signal_and_timeout() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("LIVE_SHUTDOWN_STREAM").to_uppercase();
    let subject = format!("live.{}.shutdown", unique_name("consumer-shutdown-subject"));
    let durable = unique_name("shutdown-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live shutdown consumer test: {err}");
            return Ok(());
        }
    };

    let probe = ShutdownProbe::default();
    let unique = unique_name("shutdown-consumer");
    let group = format!("live.{unique}");
    let status_subject = format!("{group}.status");

    let mut status_endpoint = LiveShutdownConsumerService::status_endpoint();
    status_endpoint.group = group;

    let mut consumer = LiveShutdownConsumerService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable;
    consumer.config.filter_subject = subject.clone();

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .state(probe.clone())
            .with_shutdown_drain_timeout(Duration::from_secs(2))
            .service_def(service_definition(
                unique_name("shutdown-consumer-service"),
                vec![status_endpoint],
                vec![consumer],
            )),
    );

    request_expected_replies(&client, &status_subject, 1).await?;

    jetstream
        .publish(subject.clone(), "hello".into())
        .await
        .context("failed to publish shutdown consumer message")?
        .await
        .context("failed to await shutdown consumer publish ack")?;

    timeout(Duration::from_secs(5), probe.started.notified())
        .await
        .context("timed out waiting for consumer handler to start")?;

    let _ = shutdown_tx.send(());

    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for shutdown consumer app")?
        .context("shutdown consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    assert_eq!(
        probe.observation(),
        ShutdownObservation {
            requested: true,
            drain_timeout: Some(Duration::from_secs(2)),
            deadline_present: true,
        }
    );
    app_result?;
    Ok(())
}

async fn connect_live_nats() -> Option<async_nats::Client> {
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());

    match async_nats::connect(&nats_url).await {
        Ok(client) => Some(client),
        Err(err) => {
            eprintln!(
                "skipping live runtime tests because NATS is unavailable at {nats_url}: {err}"
            );
            None
        }
    }
}

#[tokio::test]
async fn live_consumer_promotes_concurrency_limit_to_max_ack_pending() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("PROMOTE_LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.promote", unique_name("consumer-promote-subject"));
    let durable = unique_name("promote-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live promote test: {err}");
            return Ok(());
        }
    };

    let probe = ConsumerProbe::default();
    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable.clone();
    consumer.config.filter_subject = subject.clone();
    // set server consumer max_ack_pending higher than the app default to exercise promotion
    consumer.config.max_ack_pending = 5;

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .with_default_concurrency_limit(1)
            .state(probe.clone())
            .service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let publish_result = async {
        jetstream
            .publish(subject.clone(), "hello".into())
            .await
            .context("failed to publish live JetStream test message")?
            .await
            .context("failed to await publish ack")?;

        timeout(Duration::from_secs(5), probe.processed.notified())
            .await
            .context("timed out waiting for live consumer to process message")?;

        Ok::<_, anyhow::Error>(())
    }
    .await;

    let _ = shutdown_tx.send(());
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app shutdown")?
        .context("consumer app task failed")?;
    let _ = jetstream.delete_stream(&stream_name).await;

    publish_result?;
    app_result?;
    assert_eq!(probe.hits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn live_consumer_configured_concurrency_exceeding_max_ack_pending_errors() -> Result<()> {
    let Some(client) = connect_live_nats().await else {
        return Ok(());
    };

    let jetstream = async_nats::jetstream::new(client.clone());
    let stream_name = unique_name("BAD_LIVE_STREAM").to_uppercase();
    let subject = format!("live.{}.bad", unique_name("consumer-bad-subject"));
    let durable = unique_name("bad-durable");

    let _stream = match jetstream
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("skipping live bad-config test: {err}");
            return Ok(());
        }
    };

    let mut consumer = LiveConsumerFlowService::jobs_consumer();
    consumer.stream = stream_name.clone();
    consumer.durable = durable.clone();
    consumer.config.filter_subject = subject.clone();
    // set a small server max_ack_pending and an explicit concurrency_limit larger than it
    consumer.config.max_ack_pending = 2;
    consumer.concurrency_limit = Some(5);

    let (_shutdown_tx, app_handle) = spawn_app_until_shutdown(
        NatsApp::new(client.clone())
            .service_def(consumer_service(unique_name("consumer-service"), consumer)),
    );

    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .context("timed out waiting for consumer app")?
        .context("consumer app task failed")?;

    // Should have errored due to invalid configured concurrency limit
    let err = app_result
        .expect_err("app should have errored due to invalid configured concurrency limit");
    let err_str = err.to_string();
    assert!(
        err_str.contains("invalid configured concurrency limit"),
        "unexpected error: {err_str}"
    );

    let _ = jetstream.delete_stream(&stream_name).await;
    Ok(())
}

fn spawn_app_until_shutdown(app: NatsApp) -> (oneshot::Sender<()>, JoinHandle<Result<()>>) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        app.run_until(async move {
            let _ = shutdown_rx.await;
            Ok(())
        })
        .await
    });
    (shutdown_tx, handle)
}

fn endpoint_service(name: String, endpoint: EndpointDefinition) -> ServiceDefinition {
    service_definition(name, vec![endpoint], Vec::new())
}

fn consumer_service(name: String, consumer: ConsumerDefinition) -> ServiceDefinition {
    service_definition(name, Vec::new(), vec![consumer])
}

fn service_definition(
    name: String,
    endpoints: Vec<EndpointDefinition>,
    consumers: Vec<ConsumerDefinition>,
) -> ServiceDefinition {
    ServiceDefinition {
        metadata: ServiceMetadata::new(name, "0.1.0", "", None),
        endpoints,
        consumers,
        endpoint_info: Vec::new(),
        consumer_info: Vec::new(),
    }
}

#[cfg(feature = "client")]
struct SpawnedLiveGeneratedClientService {
    shutdown_tx: oneshot::Sender<()>,
    app_handle: JoinHandle<Result<()>>,
    prefix: String,
    #[cfg(feature = "encryption")]
    recipient: [u8; 32],
}

#[cfg(feature = "client")]
fn live_generated_client_service_definition(prefix: String) -> ServiceDefinition {
    let mut definition = LiveGeneratedClientService::definition();
    let service_name = unique_name("generated-client-service");

    definition.metadata.name = service_name.clone();
    definition.metadata.subject_prefix = Some(prefix.clone());

    for endpoint in &mut definition.endpoints {
        endpoint.service_name = service_name.clone();
        endpoint.subject_prefix = Some(prefix.clone());
    }

    definition
}

#[cfg(feature = "client")]
fn build_live_generated_client(
    client: async_nats::Client,
    prefix: String,
    #[cfg(feature = "encryption")] recipient: [u8; 32],
) -> live_generated_client_service_client::LiveGeneratedClientServiceClient {
    #[cfg(feature = "encryption")]
    {
        live_generated_client_service_client::LiveGeneratedClientServiceClient::with_prefix(
            client, prefix,
        )
        .with_recipient(recipient)
    }

    #[cfg(not(feature = "encryption"))]
    {
        live_generated_client_service_client::LiveGeneratedClientServiceClient::with_prefix(
            client, prefix,
        )
    }
}

#[cfg(all(feature = "client", feature = "napi"))]
async fn build_live_generated_napi_client(
    server: String,
    prefix: String,
    #[cfg(feature = "encryption")] recipient: [u8; 32],
) -> nats_micro::napi::Result<JsLiveGeneratedClientServiceClient, String> {
    let options = LiveGeneratedClientServiceClientConnectOptions {
        subject_prefix: Some(prefix),
        #[cfg(feature = "encryption")]
        recipient_public_key: Some(nats_micro::napi::bindgen_prelude::Buffer::from(
            recipient.to_vec(),
        )),
        ..Default::default()
    };

    JsLiveGeneratedClientServiceClient::connect(server, Some(options)).await
}

#[cfg(all(feature = "client", feature = "napi", feature = "encryption"))]
async fn build_live_generated_napi_client_without_recipient(
    server: String,
    prefix: String,
) -> nats_micro::napi::Result<JsLiveGeneratedClientServiceClient, String> {
    let options = LiveGeneratedClientServiceClientConnectOptions {
        subject_prefix: Some(prefix),
        ..Default::default()
    };

    JsLiveGeneratedClientServiceClient::connect(server, Some(options)).await
}

#[cfg(feature = "client")]
async fn wait_for_generated_client_service(
    client: &async_nats::Client,
    prefix: &str,
) -> Result<()> {
    let health_subject = format!("{prefix}.live-client.health");
    let _ = request_expected_replies(client, &health_subject, 1).await?;
    Ok(())
}

#[cfg(feature = "client")]
async fn spawn_live_generated_client_service_app(
    client: async_nats::Client,
) -> Result<SpawnedLiveGeneratedClientService> {
    let prefix = unique_name("generated-client-prefix");
    let service_def = live_generated_client_service_definition(prefix.clone());

    #[cfg(feature = "encryption")]
    let keypair = ServiceKeyPair::generate();
    #[cfg(feature = "encryption")]
    let recipient = keypair.public_key_bytes();

    let mut app = NatsApp::new(client.clone()).service_def(service_def);

    #[cfg(feature = "encryption")]
    {
        app = app.state(keypair);
    }

    let (shutdown_tx, app_handle) = spawn_app_until_shutdown(app);
    wait_for_generated_client_service(&client, &prefix).await?;

    Ok(SpawnedLiveGeneratedClientService {
        shutdown_tx,
        app_handle,
        prefix,
        #[cfg(feature = "encryption")]
        recipient,
    })
}

#[cfg(feature = "client")]
async fn spawn_live_generated_client_service(
    client: async_nats::Client,
) -> Result<(
    oneshot::Sender<()>,
    JoinHandle<Result<()>>,
    live_generated_client_service_client::LiveGeneratedClientServiceClient,
)> {
    let spawned = spawn_live_generated_client_service_app(client.clone()).await?;
    let service_client = build_live_generated_client(
        client,
        spawned.prefix.clone(),
        #[cfg(feature = "encryption")]
        spawned.recipient,
    );

    Ok((spawned.shutdown_tx, spawned.app_handle, service_client))
}

async fn shutdown_app(
    shutdown_tx: oneshot::Sender<()>,
    app_handle: JoinHandle<Result<()>>,
    label: &str,
) -> Result<()> {
    let _ = shutdown_tx.send(());
    let app_result = timeout(Duration::from_secs(5), app_handle)
        .await
        .with_context(|| format!("timed out waiting for {label} app shutdown"))?
        .with_context(|| format!("{label} app task failed"))?;
    app_result?;
    Ok(())
}

async fn request_expected_replies(
    client: &async_nats::Client,
    subject: &str,
    expected: usize,
) -> Result<Vec<String>> {
    for _ in 0..20 {
        let replies = request_replies_once(client, subject, expected).await?;
        if replies.len() == expected {
            return Ok(replies);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    anyhow::bail!("timed out waiting for {expected} replies on subject `{subject}`")
}

async fn request_replies_once(
    client: &async_nats::Client,
    subject: &str,
    expected: usize,
) -> Result<Vec<String>> {
    let inbox = client.new_inbox();
    let mut replies = client
        .subscribe(inbox.clone())
        .await
        .context("failed to subscribe to reply inbox")?;

    client
        .publish_with_reply(subject.to_string(), inbox, "".into())
        .await
        .with_context(|| format!("failed to publish test request to `{subject}`"))?;
    client
        .flush()
        .await
        .context("failed to flush live test request")?;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    let mut payloads = Vec::new();

    while payloads.len() < expected {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match timeout(remaining, replies.next()).await {
            Ok(Some(message)) => payloads.push(
                String::from_utf8(message.payload.to_vec())
                    .context("reply payload was not valid UTF-8")?,
            ),
            Ok(None) | Err(_) => break,
        }
    }

    Ok(payloads)
}

fn unique_name(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}
