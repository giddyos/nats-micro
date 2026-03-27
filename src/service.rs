use crate::consumer::ConsumerDefinition;
use crate::handler::HandlerFn;

#[derive(Debug, Clone)]
pub struct ServiceMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
}

impl ServiceMetadata {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
        }
    }
}

#[derive(Clone)]
pub struct EndpointDefinition {
    pub service_name: String,
    pub service_version: String,
    pub service_description: String,
    pub group: String,
    pub subject: String,
    pub subject_template: Option<String>,
    pub queue_group: Option<String>,
    pub auth_required: bool,
    pub handler: HandlerFn,
}

impl EndpointDefinition {
    pub fn full_subject(&self) -> String {
        match (self.service_name.is_empty(), self.group.is_empty()) {
            (false, false) => format!("{}.{}.{}", self.service_name, self.group, self.subject),
            (false, true) => format!("{}.{}", self.service_name, self.subject),
            (true, false) => format!("{}.{}", self.group, self.subject),
            (true, true) => self.subject.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub type_name: String,
    pub is_subject_param: bool,
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
}

#[derive(Debug, Clone)]
pub struct ConsumerInfo {
    pub fn_name: String,
    pub stream: String,
    pub durable: String,
    pub filter_subject: String,
    pub ack_on_success: bool,
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
