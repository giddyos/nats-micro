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
- Hardened `#[service_error]` wire attributes with strict `#[code]`,
  `#[internal]`, `#[kind]`, and `#[details]` parsing, duplicate wire-kind
  validation, generic `INTERNAL_ERROR` kinds for internal variants, and
  `#[details(skip)]` / `#[details(skip_all)]` controls for public details.
- Hardened fixture manifest policy checks and wired compile/runtime fixtures
  into `scripts/check.sh`.
- Added `scripts/release-check.sh` with publish dry-runs for the workspace
  crates.
- Prepared package versions and metadata for the 1.0.0 release.
