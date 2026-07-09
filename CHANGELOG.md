# Changelog

## 1.0.0

- Added dependency facade coverage for downstream crates using `async-nats`,
  `thiserror`, `serde`, `serde_json`, `prost`, and `bytes` through
  `nats-micro`.
- Added representative downstream fixtures for service authoring, generated
  clients, runtime roundtrips, JetStream consumers, encryption, renamed
  dependencies, no-default-features, N-API, and the raw `thiserror` caveat.
- Hardened fixture manifest policy checks and wired compile/runtime fixtures
  into `scripts/check.sh`.
- Prepared package versions and metadata for the 1.0.0 release.
