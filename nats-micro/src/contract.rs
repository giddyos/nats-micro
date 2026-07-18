use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{AuthPolicy, Codec, OperationKind, ServiceContract};

/// An owned, serializable contract document for files and tooling.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContractDocument {
    pub service: ContractService,
}

/// Owned service metadata used outside the request hot path.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContractService {
    pub name: String,
    pub version: String,
    pub description: String,
    pub prefix: String,
    pub operations: Vec<ContractOperation>,
    pub consumers: Vec<ContractConsumer>,
}

/// Owned operation metadata used by contract and deployment tooling.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContractOperation {
    pub rust_name: String,
    pub kind: OperationKind,
    pub subject: String,
    pub subject_template: String,
    pub queue_group: Option<String>,
    pub request_codec: Codec,
    pub response_codec: Codec,
    pub request_encrypted: bool,
    pub response_encrypted: bool,
    pub request_type: Option<String>,
    pub response_type: Option<String>,
    pub error_type: Option<String>,
    pub auth: AuthPolicy,
    pub concurrency: usize,
    pub params: Vec<ContractParam>,
}

/// Owned subject parameter metadata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContractParam {
    pub name: String,
    pub segment: u16,
    pub rust_type: String,
}

/// Owned consumer metadata used by contract and deployment tooling.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContractConsumer {
    pub rust_name: String,
    pub stream: String,
    pub durable: String,
    pub filter_subject: String,
    pub concurrency: usize,
    pub ack_wait_ms: u64,
    pub max_deliver: i64,
    pub backoff_ms: Vec<u64>,
}

impl From<ServiceContract<'_>> for ContractDocument {
    fn from(contract: ServiceContract<'_>) -> Self {
        let service = contract.service;
        Self {
            service: ContractService {
                name: service.name.to_owned(),
                version: service.version.to_owned(),
                description: service.description.to_owned(),
                prefix: service.prefix.to_owned(),
                operations: service
                    .operations
                    .iter()
                    .map(|operation| ContractOperation {
                        rust_name: operation.rust_name.to_owned(),
                        kind: operation.kind,
                        subject: operation.subject.to_owned(),
                        subject_template: operation.subject_template.to_owned(),
                        queue_group: operation.queue_group.map(str::to_owned),
                        request_codec: operation.request_codec,
                        response_codec: operation.response_codec,
                        request_encrypted: operation.request_encrypted,
                        response_encrypted: operation.response_encrypted,
                        request_type: operation.request_type.map(str::to_owned),
                        response_type: operation.response_type.map(str::to_owned),
                        error_type: operation.error_type.map(str::to_owned),
                        auth: operation.auth,
                        concurrency: operation.concurrency,
                        params: operation
                            .params
                            .iter()
                            .map(|param| ContractParam {
                                name: param.name.to_owned(),
                                segment: param.segment,
                                rust_type: param.rust_type.to_owned(),
                            })
                            .collect(),
                    })
                    .collect(),
                consumers: service
                    .consumers
                    .iter()
                    .map(|consumer| ContractConsumer {
                        rust_name: consumer.rust_name.to_owned(),
                        stream: consumer.stream.to_owned(),
                        durable: consumer.durable.to_owned(),
                        filter_subject: consumer.filter_subject.to_owned(),
                        concurrency: consumer.concurrency,
                        ack_wait_ms: consumer.ack_wait_ms,
                        max_deliver: consumer.max_deliver,
                        backoff_ms: consumer.backoff_ms.to_vec(),
                    })
                    .collect(),
            },
        }
    }
}

impl ContractDocument {
    /// Validates invariants expected by runtime and deployment tooling.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        let service = &self.service;
        if service.name.is_empty() {
            return Err(ContractValidationError::new(
                "service name must not be empty",
            ));
        }
        if !valid_semver(&service.version) {
            return Err(ContractValidationError::new(
                "service version must be semantic version MAJOR.MINOR.PATCH",
            ));
        }

        let mut subjects = HashSet::with_capacity(service.operations.len());
        for operation in &service.operations {
            if operation.subject.is_empty() {
                return Err(ContractValidationError::new(format!(
                    "operation `{}` has an empty subject",
                    operation.rust_name
                )));
            }
            if operation.concurrency == 0 {
                return Err(ContractValidationError::new(format!(
                    "operation `{}` has zero concurrency",
                    operation.rust_name
                )));
            }
            if !subjects.insert(operation.subject.as_str()) {
                return Err(ContractValidationError::new(format!(
                    "duplicate operation subject `{}`",
                    operation.subject
                )));
            }
        }
        for consumer in &service.consumers {
            if consumer.stream.is_empty()
                || consumer.durable.is_empty()
                || consumer.filter_subject.is_empty()
            {
                return Err(ContractValidationError::new(format!(
                    "consumer `{}` requires stream, durable, and filter subject",
                    consumer.rust_name
                )));
            }
            if consumer.concurrency == 0 {
                return Err(ContractValidationError::new(format!(
                    "consumer `{}` has zero concurrency",
                    consumer.rust_name
                )));
            }
        }
        Ok(())
    }

    /// Returns all core and `JetStream` subjects in declaration order.
    #[must_use]
    pub fn subjects(&self) -> impl Iterator<Item = &str> {
        self.service
            .operations
            .iter()
            .map(|operation| operation.subject.as_str())
            .chain(
                self.service
                    .consumers
                    .iter()
                    .map(|consumer| consumer.filter_subject.as_str()),
            )
    }
}

fn valid_semver(version: &str) -> bool {
    let core = version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core);
    let mut parts = core.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(major), Some(minor), Some(patch), None)
            if [major, minor, patch]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    )
}

/// A stable validation failure for a contract document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ContractValidationError {
    message: String,
}

impl ContractValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
