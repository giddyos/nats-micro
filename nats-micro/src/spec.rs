use serde::{Deserialize, Serialize};

/// The transport behavior implemented by an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Request,
    Publish,
    Subscribe,
    Consumer,
}

/// The wire codec used by an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Json,
    Protobuf,
    Raw,
    Utf8,
    Empty,
}

/// The authentication requirement attached to an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPolicy {
    None,
    Optional,
    Required,
}

impl AuthPolicy {
    #[must_use]
    pub const fn auth_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Compile-time metadata for one subject parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub segment: u16,
    pub rust_type: &'static str,
}

/// Compile-time metadata for one service operation.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct OperationSpec {
    #[serde(skip)]
    pub service_name: &'static str,
    #[serde(skip)]
    pub service_version: &'static str,
    pub rust_name: &'static str,
    pub kind: OperationKind,
    pub subject: &'static str,
    pub subject_template: &'static str,
    pub queue_group: Option<&'static str>,
    pub request_codec: Codec,
    pub response_codec: Codec,
    pub request_encrypted: bool,
    pub response_encrypted: bool,
    pub request_type: Option<&'static str>,
    pub response_type: Option<&'static str>,
    pub error_type: Option<&'static str>,
    pub auth: AuthPolicy,
    pub concurrency: usize,
    pub params: &'static [ParamSpec],
}

/// Compile-time metadata for one `JetStream` consumer.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ConsumerSpec {
    #[serde(skip)]
    pub service_name: &'static str,
    #[serde(skip)]
    pub service_version: &'static str,
    pub rust_name: &'static str,
    pub stream: &'static str,
    pub durable: &'static str,
    pub filter_subject: &'static str,
    pub concurrency: usize,
    pub ack_wait_ms: u64,
    pub max_deliver: i64,
    pub backoff_ms: &'static [u64],
}

/// Compile-time metadata for a complete service.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceSpec {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub prefix: &'static str,
    pub operations: &'static [OperationSpec],
    pub consumers: &'static [ConsumerSpec],
}

/// An allocation-free view of a generated service contract.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceContract<'a> {
    pub service: &'a ServiceSpec,
}

impl<'a> ServiceContract<'a> {
    /// Returns the service's statically allocated operation metadata.
    #[must_use]
    pub const fn operations(self) -> &'a [OperationSpec] {
        self.service.operations
    }

    /// Returns the service's statically allocated consumer metadata.
    #[must_use]
    pub const fn consumers(self) -> &'a [ConsumerSpec] {
        self.service.consumers
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthPolicy, Codec, ConsumerSpec, OperationKind, OperationSpec, ParamSpec, ServiceSpec,
    };

    const PARAMS: &[ParamSpec] = &[ParamSpec {
        name: "user_id",
        segment: 3,
        rust_type: "&str",
    }];
    const OPERATIONS: &[OperationSpec] = &[OperationSpec {
        service_name: "users",
        service_version: "2.0.0",
        rust_name: "find_user",
        kind: OperationKind::Request,
        subject: "users.v1.users.*",
        subject_template: "users.v1.users.{user_id}",
        queue_group: Some("users"),
        request_codec: Codec::Raw,
        response_codec: Codec::Utf8,
        request_encrypted: false,
        response_encrypted: false,
        request_type: Some("&[u8]"),
        response_type: Some("&str"),
        error_type: None,
        auth: AuthPolicy::None,
        concurrency: 32,
        params: PARAMS,
    }];
    const CONSUMERS: &[ConsumerSpec] = &[ConsumerSpec {
        service_name: "users",
        service_version: "2.0.0",
        rust_name: "project_user",
        stream: "USERS",
        durable: "users-projector",
        filter_subject: "users.events.>",
        concurrency: 8,
        ack_wait_ms: 30_000,
        max_deliver: 5,
        backoff_ms: &[100, 1_000],
    }];
    const SERVICE: ServiceSpec = ServiceSpec {
        name: "users",
        version: "2.0.0",
        description: "user service",
        prefix: "tenant",
        operations: OPERATIONS,
        consumers: CONSUMERS,
    };

    #[test]
    fn metadata_is_static_and_copy() {
        let copied = SERVICE;
        assert_eq!(copied.operations[0].params[0].segment, 3);
        assert_eq!(copied.consumers[0].backoff_ms, &[100, 1_000]);
    }
}
