# Downstream fixtures

These fixtures compile or run as independent downstream crates. They intentionally
do not belong to the root workspace.

| Fixture | Purpose | Direct async-nats? | Direct thiserror? | Runtime NATS? |
|---|---|---:|---:|---:|
| service-authoring-basic | Service macros, extractors, responses, service_error | no | no | no |
| generated-client-app | Generated client API and facade aliases | no | no | no |
| runtime-roundtrip | Real NatsApp + generated client roundtrip | no | no | yes |
| consumer-jetstream | Consumer authoring and JetStream config | no | no | optional/yes |
| encryption-client | Encrypted payload/header client surface | no | no | no |
| renamed-dependency-full | Renamed nats-micro macro path resolution | no | no | no |
| no-default-features-service | Server/service authoring with no default features | no | no | no |
| napi-surface | NAPI/object macro public surface | no | no | no |
| raw-thiserror-direct-dep | Raw thiserror caveat | no | yes | no |
| existing-thiserror-service-error | service_error on existing thiserror enums | no | yes | no |

Public DTOs in fixtures should use `#[serde(crate = "nats_micro::serde")]` so downstream crates do not need a direct `serde` dependency for derives. For protobuf DTOs, `prost` derives still emit `::prost` paths, so fixtures that avoid a direct `prost` dependency implement `nats_micro::prost::Message` manually through the facade.
