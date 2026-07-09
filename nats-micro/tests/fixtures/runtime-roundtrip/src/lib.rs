#![allow(clippy::unused_async)]

use nats_micro::{
    Headers, Json, NatsErrorResponse, Payload, RequestId, State, Subject, SubjectParam, service,
    service_error, service_handlers,
};

#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct CreateUserRequest {
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct UserCreated {
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct RequestSnapshot {
    pub request_id: String,
    pub subject: String,
    pub trace_id: Option<String>,
    pub tenant_id: Option<String>,
    pub shard: String,
}

#[derive(Clone, Default)]
pub struct AppState {
    pub shard: String,
}

#[service_error]
pub enum UserServiceError {
    #[code(409)]
    #[error("email {email} already exists")]
    EmailExists { email: String },
}

#[service(name = "roundtrip-users", version = "1.0.0")]
pub struct UserService;

#[service_handlers]
impl UserService {
    #[endpoint(subject = "health")]
    async fn health() -> Result<String, NatsErrorResponse> {
        Ok("ok".to_string())
    }

    #[endpoint(subject = "users", group = "accounts")]
    async fn create_user(
        payload: Payload<Json<CreateUserRequest>>,
        state: State<AppState>,
    ) -> Result<Json<UserCreated>, UserServiceError> {
        if payload.email == "exists@example.com" {
            return Err(UserServiceError::EmailExists {
                email: payload.email.clone(),
            });
        }

        Ok(Json(UserCreated {
            user_id: format!("{}:{}", state.shard, payload.email),
        }))
    }

    #[endpoint(subject = "users.{user_id}.profile", group = "accounts")]
    async fn profile(user_id: SubjectParam<String>) -> Result<String, NatsErrorResponse> {
        Ok(format!("profile:{}", user_id.into_inner()))
    }

    #[endpoint(subject = "inspect", group = "accounts")]
    async fn inspect(
        headers: Headers,
        request_id: RequestId,
        subject: Subject,
        state: State<AppState>,
    ) -> Result<Json<RequestSnapshot>, NatsErrorResponse> {
        Ok(Json(RequestSnapshot {
            request_id: request_id.into_inner(),
            subject: subject.into_inner(),
            trace_id: headers.get("x-trace-id").map(|value| value.as_str().to_string()),
            tenant_id: headers.get("x-tenant-id").map(|value| value.as_str().to_string()),
            shard: state.shard.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{future::pending, time::Duration};

    use anyhow::{Context, Result};
    use nats_micro::{
        ClientCallOptions, ClientError, NatsApp, NatsAppConfig, WorkerFailurePolicy,
    };
    use tokio::{task::JoinHandle, time::timeout};

    use super::*;

    fn spawn_app(app: NatsApp) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            app.run_until(async {
                pending::<()>().await;
                Ok(())
            })
            .await
        })
    }

    async fn wait_for_ready(
        client: &user_service_client::UserServiceClient,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if matches!(client.health().await.as_deref(), Ok("ok")) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for runtime fixture service readiness");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn generated_client_round_trips_against_nats_app() -> Result<()> {
        let server = nats_server::run_basic_server();
        let server_url = server.client_url();
        let connected = nats_micro::connect(server_url.clone(), None).await?;

        let app = NatsApp::new(connected.client.clone())
            .with_config(
                NatsAppConfig::new()
                    .with_default_concurrency_limit(8)
                    .with_worker_failure_policy(WorkerFailurePolicy::ShutdownApp),
            )
            .state(AppState {
                shard: "shard-a".to_string(),
            })
            .service::<UserService>();
        let app_handle = spawn_app(app);

        let service_client =
            user_service_client::UserServiceClient::connect(server_url, None).await?;
        wait_for_ready(&service_client).await?;

        let created = service_client
            .create_user(&CreateUserRequest {
                email: "new@example.com".to_string(),
            })
            .await
            .context("create_user failed")?;
        assert_eq!(
            created,
            UserCreated {
                user_id: "shard-a:new@example.com".to_string()
            }
        );

        let profile = service_client
            .profile(&"user-1".to_string())
            .await
            .context("profile failed")?;
        assert_eq!(profile, "profile:user-1");

        let snapshot = service_client
            .inspect_with(
                ClientCallOptions::new()
                    .header("x-request-id", "req-runtime")
                    .header("x-trace-id", "trace-runtime")
                    .header("x-tenant-id", "tenant-a"),
            )
            .await
            .context("inspect failed")?;
        assert_eq!(snapshot.request_id, "req-runtime");
        assert_eq!(snapshot.trace_id.as_deref(), Some("trace-runtime"));
        assert_eq!(snapshot.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(snapshot.subject, "roundtrip-users.v1.accounts.inspect");
        assert_eq!(snapshot.shard, "shard-a");

        let error = service_client
            .create_user(&CreateUserRequest {
                email: "exists@example.com".to_string(),
            })
            .await
            .expect_err("existing email should return a service error");
        match error {
            ClientError::Service {
                error: UserServiceError::EmailExists { email },
                response,
            } => {
                assert_eq!(email, "exists@example.com");
                assert_eq!(response.kind, "EMAIL_EXISTS");
            }
            other => panic!("expected typed service error, got {other:?}"),
        }

        let unknown: ClientError<UserServiceError> = ClientError::from_service_response(
            NatsErrorResponse::new(499, "UNKNOWN_SERVICE_ERROR", "unknown", "req-unknown"),
        );
        assert!(matches!(unknown, ClientError::ServiceResponse(_)));

        connected.client.drain().await?;
        let app_result = timeout(Duration::from_secs(5), app_handle)
            .await
            .context("timed out waiting for runtime fixture app shutdown")?
            .context("runtime fixture app task failed")?;
        assert!(app_result.is_err());

        Ok(())
    }
}
