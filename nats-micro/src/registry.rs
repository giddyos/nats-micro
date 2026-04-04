use crate::service::ServiceDefinition;

pub struct ServiceRegistration {
    pub constructor: fn() -> ServiceDefinition,
}

inventory::collect!(ServiceRegistration);
