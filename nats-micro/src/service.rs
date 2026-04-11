use crate::consumer::ConsumerDefinition;
use crate::handler::HandlerFn;

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

#[derive(Debug, Clone)]
pub struct ServiceMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub subject_prefix: Option<String>,
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
    pub auth_required: bool,
    pub concurrency_limit: Option<u64>,
    pub handler: HandlerFn,
}

impl EndpointDefinition {
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

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub type_name: String,
    pub is_subject_param: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadEncoding {
    Json,
    Proto,
    Serde,
    Raw,
}

#[derive(Debug, Clone)]
pub struct PayloadMeta {
    pub encoding: PayloadEncoding,
    pub encrypted: bool,
    pub inner_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseEncoding {
    Json,
    Proto,
    Serde,
    Raw,
    Unit,
}

#[derive(Debug, Clone)]
pub struct ResponseMeta {
    pub encoding: ResponseEncoding,
    pub encrypted: bool,
    pub inner_type: String,
}

#[derive(Debug, Clone)]
pub struct EndpointInfo {
    pub fn_name: String,
    pub subject_template: String,
    pub subject_pattern: String,
    pub group: String,
    pub queue_group: Option<String>,
    pub auth_required: bool,
    pub concurrency_limit: Option<u64>,
    pub params: Vec<ParamInfo>,
    pub payload_meta: Option<PayloadMeta>,
    pub response_meta: ResponseMeta,
}

#[derive(Debug, Clone)]
pub struct ConsumerInfo {
    pub fn_name: String,
    pub stream: String,
    pub durable: String,
    pub auth_required: bool,
    pub concurrency_limit: Option<u64>,
    pub params: Vec<ParamInfo>,
}

#[derive(Clone)]
pub struct ServiceDefinition {
    pub metadata: ServiceMetadata,
    pub endpoints: Vec<EndpointDefinition>,
    pub consumers: Vec<ConsumerDefinition>,
    pub endpoint_info: Vec<EndpointInfo>,
    pub consumer_info: Vec<ConsumerInfo>,
}

pub trait NatsService: Send + Sync + 'static {
    fn definition() -> ServiceDefinition;
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
