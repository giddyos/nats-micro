use std::time::Duration;

use anyhow::{Context, Result};
use nats_micro::{
    App, AppState, Auth, AuthError, Body, Bytes, ConsumerAction, ErrorReply, FromRequestMeta,
    Headers, LocalService, Request, RequestMeta, Response, StateRef, Text, message, service,
    service_error,
};
use tokio::sync::oneshot;

#[message(rename_all = "camelCase")]
pub struct CreateUser<'a> {
    #[serde(borrow)]
    pub email: &'a str,
    #[serde(borrow)]
    pub display_name: Option<&'a str>,
}

#[message]
pub struct User {
    pub id: String,
    pub email: String,
}

#[service(name = "probe", version = "2.0.0")]
impl ProbeService {
    #[request("ping")]
    async fn ping() -> User {
        User {
            id: "live".to_owned(),
            email: "probe@example.com".to_owned(),
        }
    }
}

#[allow(dead_code)]
async fn canonical_stateless_application() -> Result<()> {
    App::stateless().serve(ProbeService).run().await
}

#[service_error]
pub enum UserError {
    #[error(code = 404, message = "user {id} was not found")]
    NotFound { id: String },

    #[internal]
    Database(#[from] std::io::Error),
}

pub struct UserRepository {
    cached: User,
}

#[derive(AppState)]
pub struct UserState {
    #[state]
    users: UserRepository,
}

pub struct Claims {
    subject: String,
}

impl FromRequestMeta for Claims {
    async fn from_request_meta(meta: RequestMeta<'_>) -> Result<Self, AuthError> {
        Ok(Self {
            subject: meta.subject.to_owned(),
        })
    }
}

#[service(
    name = "users",
    version = "1.4.7",
    state = UserState,
    defaults(concurrency = 8, queue = "users-v1", codec = json)
)]
impl UserService {
    #[request]
    async fn health() -> &'static str {
        "ok"
    }

    #[request("users.{user_id}")]
    async fn get(
        state: &UserState,
        user_id: &str,
        #[header("x-tenant-id")] tenant_id: Option<&str>,
    ) -> User {
        User {
            id: format!("{user_id}:{}", tenant_id.unwrap_or("public")),
            email: state.users.cached.email.clone(),
        }
    }

    #[request("create")]
    async fn create(input: CreateUser<'_>) -> User {
        User {
            id: "new".to_owned(),
            email: input.email.to_owned(),
        }
    }

    #[request("inspect")]
    async fn inspect(headers: Headers<'_>, meta: RequestMeta<'_>) -> &'static str {
        let _ = headers.get("x-trace-id");
        let _ = meta.subject;
        "inspected"
    }

    #[request("raw")]
    async fn raw(body: Body<'_>) -> Bytes {
        Bytes::copy_from_slice(body.0)
    }

    #[request("text")]
    async fn text(text: Text<'_>) -> &'static str {
        let _ = text.0;
        "text"
    }

    #[request("optional")]
    async fn optional(input: Option<CreateUser<'_>>) -> Option<User> {
        input.map(|input| User {
            id: "optional".to_owned(),
            email: input.email.to_owned(),
        })
    }

    #[request("cached", response = User)]
    async fn cached(state: &UserState) -> Result<&User, ErrorReply> {
        Ok(&state.users.cached)
    }

    #[request("projected")]
    async fn projected(users: StateRef<'_, UserRepository>) -> User {
        User {
            id: users.cached.id.clone(),
            email: users.cached.email.clone(),
        }
    }

    #[request("me")]
    async fn me(auth: Auth<Claims>) -> User {
        User {
            id: auth.subject.clone(),
            email: "claims@example.com".to_owned(),
        }
    }

    #[request("maybe-me")]
    async fn maybe_me(auth: Option<Auth<Claims>>) -> Option<User> {
        auth.map(|auth| User {
            id: auth.subject.clone(),
            email: "claims@example.com".to_owned(),
        })
    }

    #[request("fail")]
    async fn fail() -> Result<User, UserError> {
        Err(UserError::NotFound {
            id: "missing".to_owned(),
        })
    }

    #[request("fail-internal")]
    async fn fail_internal() -> Result<User, UserError> {
        Err(UserError::Database(std::io::Error::other(
            "private database details",
        )))
    }

    #[publish("events.{user_id}")]
    async fn changed(user_id: &str, event: User) {
        let _ = (user_id, event);
    }

    #[subscribe("users.events")]
    async fn observe(body: Body<'_>) {
        let _ = body;
    }

    #[consumer(
        stream = "USERS",
        durable = "users-projector",
        filter = "users.events",
        concurrency = 2,
        ack_wait = "3s",
        backoff = ["100ms", "1s"]
    )]
    async fn project(body: Body<'_>) -> ConsumerAction {
        let _ = body;
        ConsumerAction::Ack
    }
}

fn state() -> UserState {
    UserState {
        users: UserRepository {
            cached: User {
                id: "cached".to_owned(),
                email: "cached@example.com".to_owned(),
            },
        },
    }
}

#[tokio::test]
async fn generated_service_uses_static_metadata_and_local_dispatch() {
    assert_eq!(UserService::SPEC.name, "users");
    assert_eq!(UserService::SPEC.version, "1.4.7");
    assert_eq!(UserService::SPEC.operations.len(), 15);
    assert_eq!(UserService::SPEC.consumers.len(), 1);
    assert_eq!(UserService::GET.spec.subject, "users.v1.users.*");
    assert_eq!(UserService::GET.spec.params[0].segment, 3);
    assert_eq!(UserService::PROJECTED.spec.concurrency, 8);

    let state = state();
    let response = <UserService as LocalService<UserState>>::dispatch_local(
        &state,
        UserService::CREATE.index,
        Request::new(
            "users.v1.create",
            Some("_INBOX.test"),
            br#"{"email":"ada@example.com","displayName":"Ada"}"#,
            None,
        ),
    )
    .await
    .unwrap();
    let Response::Payload(payload) = response else {
        panic!("expected JSON payload");
    };
    let user: User = serde_json::from_slice(&payload).unwrap();
    assert_eq!(user.email, "ada@example.com");

    let client = UserServiceClient::new(());
    assert_eq!(client.transport(), &());
    assert!(UserService::contract_json().unwrap().contains("\"users\""));

    let error = <UserService as LocalService<UserState>>::dispatch_local(
        &state,
        UserService::FAIL.index,
        Request::new("users.v1.fail", Some("_INBOX.test"), b"", None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, 404);
    assert_eq!(error.kind, "NOT_FOUND");

    let internal = <UserService as LocalService<UserState>>::dispatch_local(
        &state,
        UserService::FAIL_INTERNAL.index,
        Request::new("users.v1.fail-internal", Some("_INBOX.test"), b"", None),
    )
    .await
    .unwrap_err();
    assert_eq!(internal.code, 500);
    assert_eq!(internal.kind, "INTERNAL_ERROR");
    assert_eq!(internal.message, "an internal error occurred");
    assert!(!String::from_utf8_lossy(&internal.payload).contains("private database details"));
}

#[tokio::test]
async fn generated_stateless_endpoint_round_trips_over_live_nats() -> Result<()> {
    let server = nats_server::run_basic_server();
    let client = nats_micro::async_nats::connect(server.client_url()).await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let app = tokio::spawn(
        App::stateless()
            .with_client(client.clone())
            .serve(ProbeService)
            .run_until(async {
                shutdown_rx
                    .await
                    .context("live test shutdown channel closed")
            }),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = client
        .request(ProbeService::PING.spec.subject, Bytes::new())
        .await?;
    let user: User = serde_json::from_slice(&response.payload)?;
    assert_eq!(user.id, "live");
    assert_eq!(user.email, "probe@example.com");

    shutdown_tx
        .send(())
        .map_err(|()| anyhow::anyhow!("generated application exited before shutdown"))?;
    tokio::time::timeout(Duration::from_secs(5), app)
        .await
        .context("generated application did not drain")???;
    Ok(())
}
