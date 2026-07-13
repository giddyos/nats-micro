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

With the default `encryption` feature enabled, generated clients accept an
optional recipient public key. With `default-features = false` and
`features = ["client"]`, generated clients only take `NatsClient`.

## Service Errors

Self-contained mode does not require a direct `thiserror` dependency:

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

Existing `thiserror` enums can keep their derive:

```rust
use thiserror::Error;

#[nats_micro::service_error]
#[derive(Debug, Error)]
pub enum ExistingError {
    #[code(404)]
    #[error("user {id} was not found")]
    NotFound { id: String },

    #[error("database failed")]
    Database(#[from] std::io::Error),
}
```

Without an `Error` derive, `#[service_error]` owns `Display`,
`std::error::Error`, and `From` for `#[from]` fields. With an `Error` derive,
`thiserror` owns those local error impls and `#[service_error]` only adds NATS
wire conversion. A direct `thiserror` dependency is only needed when you choose
the existing-derive mode. `#[service_error]` must be placed before
`#[derive(Error)]` when reusing an existing `thiserror` enum.

Public coded variants are serialized with their display message and details by
default. Details are part of the wire protocol, so do not put secrets in public
error fields. Use `#[internal]` for private failures, `#[details(skip)]` on a
field, or `#[details(skip_all)]` on a variant to omit fields from details:

```rust
#[nats_micro::service_error]
pub enum LoginError {
    #[code(401)]
    #[error("invalid login for {username}")]
    InvalidLogin {
        username: String,
        #[details(skip)]
        attempted_password: String,
    },
}
```

Any skipped field, including `#[details(skip_all)]`, prevents typed client
reconstruction for that variant. The client receives `ClientError::ServiceResponse`
with the original NATS error response instead.

Internal `#[from]` and transparent variants default to HTTP 500 with
`"an internal error occurred"` and `INTERNAL_ERROR` unless a `#[code(...)]`,
`#[kind("...")]`, or `#[internal(expose_kind)]` override is supplied.
Custom `#[kind("...")]` values must match `^[A-Z][A-Z0-9_]*$`. When N-API
service error enums are generated, JavaScript enum member names follow Rust
variant names, and enum string values follow the wire `kind`.

## Dependency Ownership

Normal downstream crates do not need direct dependencies on `async-nats`,
`thiserror`, `serde`, `serde_json`, `prost`, or `bytes`. Use the public facade:
`nats_micro::NatsClient`, `nats_micro::async_nats`,
`nats_micro::serde`, `nats_micro::serde_json`, `nats_micro::prost`, and
`nats_micro::Bytes`.

Raw `#[derive(nats_micro::Error)]` is the caveat: it follows `thiserror`
derive internals and may require a direct `thiserror` dependency or crate alias
for non-service local error types. Normal `#[nats_micro::service_error]` users
still do not need direct `thiserror`.

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
bash scripts/release-check.sh
```

Downstream fixture coverage is documented in
`nats-micro/tests/fixtures/README.md`.
