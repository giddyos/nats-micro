use crate::service::ServiceDefinition;

pub struct ServiceRegistration {
    pub constructor: fn() -> ServiceDefinition,
}

inventory::collect!(ServiceRegistration);

pub fn registered_services() -> Vec<ServiceDefinition> {
    inventory::iter::<ServiceRegistration>
        .into_iter()
        .map(|registration| (registration.constructor)())
        .collect()
}
