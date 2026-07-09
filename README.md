# nats-micro

`nats-micro` is a Rust framework for building NATS-backed services with
attribute macros, generated clients, typed service errors, JetStream consumers,
and optional encrypted payloads and headers.

## Minimal Service

```rust
use nats_micro::{Json, NatsErrorResponse, Payload, service, service_handlers};

#[derive(nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct CreateUser {
    pub email: String,
}

#[service(name = "users", version = "1.0.0")]
pub struct UserService;

#[service_handlers]
impl UserService {
    #[endpoint(subject = "create", group = "accounts")]
    async fn create(
        payload: Payload<Json<CreateUser>>,
    ) -> Result<String, NatsErrorResponse> {
        Ok(format!("created:{}", payload.email))
    }
}
```

## Generated Client

```rust
# async fn example(client: nats_micro::NatsClient) -> Result<(), nats_micro::ClientError<nats_micro::NatsErrorResponse>> {
# use nats_micro::{Json, NatsErrorResponse, Payload, service, service_handlers};
# #[derive(nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
# #[serde(crate = "nats_micro::serde")]
# pub struct CreateUser { pub email: String }
# #[service(name = "users", version = "1.0.0")]
# pub struct UserService;
# #[service_handlers]
# impl UserService {
#     #[endpoint(subject = "create", group = "accounts")]
#     async fn create(payload: Payload<Json<CreateUser>>) -> Result<String, NatsErrorResponse> {
#         Ok(format!("created:{}", payload.email))
#     }
# }
let users = user_service_client::UserServiceClient::new(client, None);
let id = users.create(&CreateUser {
    email: "a@example.com".to_string(),
}).await?;
# let _ = id;
# Ok(())
# }
```

## Service Errors

```rust
#[nats_micro::service_error]
pub enum CreateUserError {
    #[code(409)]
    #[error("email {email} already exists")]
    EmailExists { email: String },

    #[error("storage failed")]
    Storage(#[from] std::io::Error),
}
```

Public coded variants are serialized with their display message and details.
Internal `#[from]` and transparent variants default to HTTP 500 with
`"an internal error occurred"` unless a `#[code(...)]` is supplied.

## Dependency Ownership

Normal downstream crates do not need direct dependencies on `async-nats`,
`thiserror`, `serde`, `serde_json`, `prost`, or `bytes`. Use the public facade:
`nats_micro::NatsClient`, `nats_micro::async_nats`,
`nats_micro::serde`, `nats_micro::serde_json`, `nats_micro::prost`, and
`nats_micro::Bytes`.

Raw `#[derive(nats_micro::Error)]` is the caveat: it follows `thiserror`
derive internals and may require a direct `thiserror` dependency. Prefer
`#[nats_micro::service_error]` for service errors.

## Features

| Feature | Default? | Purpose |
|---|---:|---|
| `client` | yes | Generated Rust clients, `connect`, `ClientCallOptions` |
| `encryption` | yes | Encrypted payloads and encrypted client headers |
| `napi` | no | N-API object and generated JavaScript client surface |

Useful checks:

```bash
cargo check -p nats-micro --no-default-features
cargo check -p nats-micro --no-default-features --features client
cargo check -p nats-micro --no-default-features --features client,encryption
bash scripts/check.sh
NATS_MICRO_REQUIRE_NATS_SERVER=1 bash scripts/check.sh
```

Downstream fixture coverage is documented in
`nats-micro/tests/fixtures/README.md`.
