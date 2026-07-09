# Changelog

## 1.0.0

- Added dependency facade coverage for downstream crates using `async-nats`,
  `thiserror`, `serde`, `serde_json`, `prost`, and `bytes` through
  `nats-micro`.
- Added representative downstream fixtures for service authoring, generated
  clients, runtime roundtrips, JetStream consumers, encryption, renamed
  dependencies, no-default-features, N-API, and the raw `thiserror` caveat.
- Added `#[service_error]` interop for existing `thiserror::Error` enums so
  projects can keep local `Display`, `Error`, `source`, `From`, and transparent
  behavior while adding NATS wire conversion.
- Hardened fixture manifest policy checks and wired compile/runtime fixtures
  into `scripts/check.sh`.
- Prepared package versions and metadata for the 1.0.0 release.
