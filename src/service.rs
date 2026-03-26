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
        if self.group.is_empty() {
            self.subject.clone()
        } else {
            format!("{}.{}", self.group, self.subject)
        }
    }
}
