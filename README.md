# nats-micro

`nats-micro` is a static, typed Rust framework for NATS request/reply,
publish/subscribe, and JetStream consumers. A `#[service]` impl generates its
concrete dispatch, client, contract, and startup code at compile time.

## Service and client

```rust
use nats_micro::{App, message, service, service_error};

#[message]
pub struct CreateUser<'a> {
    pub email: &'a str,
}

#[message]
pub struct User {
    pub id: String,
    pub email: String,
}

#[service_error]
pub enum UserError {
    #[error(code = 409, message = "email {email} already exists")]
    EmailExists { email: String },
}

#[service(name = "users", version = "2.0.0")]
impl UserService {
    #[request("create")]
    async fn create(input: CreateUser<'_>) -> Result<User, UserError> {
        Ok(User {
            id: "user-1".to_owned(),
            email: input.email.to_owned(),
        })
    }
}

# async fn run() -> nats_micro::Result<()> {
App::stateless().serve(UserService).run().await
# }
```

Generated clients accept any `ClientTransport`, so the same API works over
live NATS and the in-memory test transport:

```rust
# use nats_micro::{message, service, testing::TestApp};
# #[message] pub struct Ping<'a> { pub value: &'a str }
# #[message] pub struct Pong { pub value: String }
# #[service(name = "ping", version = "1.0.0")]
# impl PingService {
#   #[request("ping")]
#   async fn ping(input: Ping<'_>) -> Pong { Pong { value: input.value.to_owned() } }
# }
# async fn run() -> nats_micro::Result<()> {
let app = TestApp::stateless().serve(PingService).start();
let pong = app
    .client::<PingService>()
    .ping(&Ping { value: "hello" })
    .await?;
# let _ = pong;
# Ok(())
# }
```

## Runtime model

Incoming requests borrow subjects, replies, headers, and payloads directly from
the transport frame. Generated endpoint and consumer marker types call concrete
service methods; there is no runtime handler registry, boxed handler future,
type-indexed state map, or owned request conversion.

`ServiceContract<'a>` is the borrowed runtime contract. Convert it to
`ContractDocument` only for owned tooling workflows such as CLI rendering and
deployment metadata generation.

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `macros` | yes | Service, message, application, and error macros |
| `client` | yes | Generated typed clients and NATS transport |
| `json` | yes | JSON request and response codecs |
| `protobuf` | no | Protobuf request and response codecs |
| `encryption` | no | Encrypted payloads and header overlays |
| `telemetry` | no | Tracing-backed metrics and lifecycle events |
| `test-util` | no | In-memory full-wire application testing |
| `test-jetstream` | no | Deterministic JetStream semantics simulator |
| `live-test` | no | Managed real-`nats-server` test harness |
| `napi` | no | Generated N-API clients |

The crate re-exports its public wire dependencies, including `serde`,
`serde_json`, `bytes`, `async_nats`, and—when enabled—`prost`.

## Validation

```bash
cargo check -p nats-micro --no-default-features
cargo test -p nats-micro
cargo test -p nats-micro --all-features
bash scripts/check.sh
bash scripts/release-check.sh
```

Performance and binary-inspection workflows are documented in
[`docs/performance.md`](docs/performance.md).
