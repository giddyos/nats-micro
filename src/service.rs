use crate::consumer::ConsumerDefinition;
use crate::handler::HandlerFn;

pub fn build_subject(prefix: Option<&str>, group: &str, subject: &str) -> String {
    match (prefix.filter(|value| !value.is_empty()), group.is_empty()) {
        (Some(prefix), false) => format!("{}.{}.{}", prefix, group, subject),
        (Some(prefix), true) => format!("{}.{}", prefix, subject),
        (None, false) => format!("{}.{}", group, subject),
        (None, true) => subject.to_string(),
    }
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
    pub handler: HandlerFn,
}

impl EndpointDefinition {
    pub fn full_subject(&self) -> String {
        build_subject(self.subject_prefix.as_deref(), &self.group, &self.subject)
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
        assert_eq!(build_subject(Some("api"), "math", "sum"), "api.math.sum");
        assert_eq!(build_subject(Some("api"), "", "health"), "api.health");
    }

    #[test]
    fn builds_subject_without_prefix() {
        assert_eq!(build_subject(None, "math", "sum"), "math.sum");
        assert_eq!(build_subject(None, "", "health"), "health");
    }
}
