use std::{future::Future, sync::Arc};

use crate::consumer::ConsumerDefinition;
use crate::handler::HandlerFn;
use crate::spec::AuthPolicy;
use serde::Serialize;
use tokio::sync::watch;

/// A generated compile-time service definition.
pub trait StaticService<S>: Send + Sync + 'static {
    const SPEC: crate::ServiceSpec;
}

/// Static startup hook emitted by `#[service]`.
///
/// The concrete future registers and drives generated endpoint types directly.
#[doc(hidden)]
pub trait RunnableService<S>: StaticService<S> + Copy {
    fn run_requests(
        self,
        state: Arc<S>,
        client: crate::NatsClient,
        shutdown: watch::Receiver<crate::ShutdownState>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// Generated local routing over the same concrete endpoint dispatch functions.
pub trait LocalService<S>: StaticService<S> {
    fn dispatch_local<'a>(
        state: &'a S,
        operation: usize,
        request: crate::Request<'a>,
    ) -> impl Future<Output = crate::DispatchResult> + Send + 'a;
}

/// Static metadata for a generated operation marker.
#[derive(Debug, Clone, Copy)]
pub struct OperationMarker {
    pub index: usize,
    pub spec: &'static crate::OperationSpec,
}

/// A generated publish-only operation.
pub trait PublishOperation: Send + Sync + 'static {
    const SPEC: crate::OperationSpec;
}

#[must_use]
pub fn build_subject(prefix: Option<&str>, version: &str, group: &str, subject: &str) -> String {
    let prefix = prefix.filter(|value| !value.is_empty());
    let major = version
        .split('.')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or("0");

    let mut full_subject = String::with_capacity(
        prefix.map_or(0, |prefix| prefix.len() + 1)
            + 1
            + major.len()
            + usize::from(!group.is_empty()) * (group.len() + 1)
            + usize::from(!subject.is_empty()) * (subject.len() + 1),
    );

    if let Some(prefix) = prefix {
        full_subject.push_str(prefix);
        full_subject.push('.');
    }

    full_subject.push('v');
    full_subject.push_str(major);

    if !group.is_empty() {
        full_subject.push('.');
        full_subject.push_str(group);
    }

    if !subject.is_empty() {
        full_subject.push('.');
        full_subject.push_str(subject);
    }

    full_subject
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub subject_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EndpointDescriptor {
    pub subject_prefix: Option<String>,
    pub service_version: String,
    pub group: String,
    pub subject_template: String,
    pub subject_pattern: String,
}

impl EndpointDescriptor {
    #[must_use]
    pub fn template(&self) -> &str {
        &self.subject_template
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.subject_pattern
    }

    #[must_use]
    pub fn full_subject(&self) -> String {
        build_subject(
            self.subject_prefix.as_deref(),
            &self.service_version,
            &self.group,
            &self.subject_pattern,
        )
    }

    #[must_use]
    pub fn full_subject_template(&self) -> String {
        build_subject(
            self.subject_prefix.as_deref(),
            &self.service_version,
            &self.group,
            &self.subject_template,
        )
    }
}

impl ServiceMetadata {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
        subject_prefix: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
            subject_prefix: subject_prefix.filter(|value| !value.is_empty()),
        }
    }
}

#[derive(Clone)]
pub struct EndpointDefinition {
    pub subject_prefix: Option<String>,
    pub service_name: String,
    pub service_version: String,
    pub service_description: String,
    pub fn_name: String,
    pub group: String,
    pub subject: String,
    pub subject_template: Option<String>,
    pub queue_group: Option<String>,
    pub auth_policy: AuthPolicy,
    pub concurrency_limit: Option<u64>,
    pub handler: HandlerFn,
}

impl EndpointDefinition {
    #[must_use]
    pub fn auth_required(&self) -> bool {
        self.auth_policy.auth_required()
    }

    pub fn full_subject(&self) -> String {
        build_subject(
            self.subject_prefix.as_deref(),
            &self.service_version,
            &self.group,
            &self.subject,
        )
    }

    pub fn full_subject_template(&self) -> Option<String> {
        self.subject_template.as_deref().map(|subject_template| {
            build_subject(
                self.subject_prefix.as_deref(),
                &self.service_version,
                &self.group,
                subject_template,
            )
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamInfo {
    pub name: String,
    pub type_name: String,
    pub is_subject_param: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadEncoding {
    Json,
    Proto,
    Serde,
    Raw,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayloadMeta {
    pub encoding: PayloadEncoding,
    pub encrypted: bool,
    pub inner_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseEncoding {
    Json,
    Proto,
    Serde,
    Raw,
    Unit,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseMeta {
    pub encoding: ResponseEncoding,
    pub encrypted: bool,
    pub inner_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointInfo {
    pub fn_name: String,
    pub subject_template: String,
    pub subject_pattern: String,
    pub group: String,
    pub queue_group: Option<String>,
    pub auth_policy: AuthPolicy,
    pub concurrency_limit: Option<u64>,
    pub params: Vec<ParamInfo>,
    pub payload_meta: Option<PayloadMeta>,
    pub response_meta: ResponseMeta,
}

impl EndpointInfo {
    #[must_use]
    pub fn auth_required(&self) -> bool {
        self.auth_policy.auth_required()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsumerInfo {
    pub fn_name: String,
    pub stream: String,
    pub durable: String,
    pub auth_policy: AuthPolicy,
    pub concurrency_limit: Option<u64>,
    pub params: Vec<ParamInfo>,
}

impl ConsumerInfo {
    #[must_use]
    pub fn auth_required(&self) -> bool {
        self.auth_policy.auth_required()
    }
}

#[derive(Clone)]
pub struct ServiceDefinition {
    pub metadata: ServiceMetadata,
    pub endpoints: Vec<EndpointDefinition>,
    pub consumers: Vec<ConsumerDefinition>,
    pub endpoint_info: Vec<EndpointInfo>,
    pub consumer_info: Vec<ConsumerInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceContract {
    pub metadata: ServiceMetadata,
    pub endpoints: Vec<EndpointInfo>,
    pub consumers: Vec<ConsumerInfo>,
}

impl ServiceDefinition {
    #[must_use]
    pub fn contract(&self) -> ServiceContract {
        ServiceContract {
            metadata: self.metadata.clone(),
            endpoints: self.endpoint_info.clone(),
            consumers: self.consumer_info.clone(),
        }
    }

    pub fn contract_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.contract())
    }
}

pub trait NatsService: Send + Sync + 'static {
    fn definition() -> ServiceDefinition;

    fn contract() -> ServiceContract {
        Self::definition().contract()
    }

    fn contract_json() -> Result<String, serde_json::Error> {
        Self::definition().contract_json()
    }
}

#[cfg(test)]
mod tests {
    use super::build_subject;

    #[test]
    fn builds_subject_with_prefix() {
        assert_eq!(
            build_subject(Some("api"), "1.2.3", "math", "sum"),
            "api.v1.math.sum"
        );
        assert_eq!(
            build_subject(Some("api"), "1.2.3", "", "health"),
            "api.v1.health"
        );
    }

    #[test]
    fn builds_subject_without_prefix() {
        assert_eq!(build_subject(None, "0.9.1", "math", "sum"), "v0.math.sum");
        assert_eq!(build_subject(None, "0.9.1", "", "health"), "v0.health");
    }

    #[test]
    fn subject_version_uses_major_only() {
        assert_eq!(
            build_subject(Some("api"), "2.0.0", "live", "jobs"),
            "api.v2.live.jobs"
        );
        assert_eq!(
            build_subject(Some("api"), "2.9.99", "live", "jobs"),
            "api.v2.live.jobs"
        );
    }
}
