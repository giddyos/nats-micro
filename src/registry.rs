use crate::{
    consumer::ConsumerDefinition,
    service::{EndpointDefinition, ServiceMetadata},
};

pub struct ServiceRegistration {
    pub constructor: fn() -> ServiceMetadata,
}

pub struct EndpointRegistration {
    pub constructor: fn() -> EndpointDefinition,
}

pub struct ConsumerRegistration {
    pub constructor: fn() -> ConsumerDefinition,
}

inventory::collect!(ServiceRegistration);
inventory::collect!(EndpointRegistration);
inventory::collect!(ConsumerRegistration);

pub fn registered_services() -> Vec<ServiceMetadata> {
    inventory::iter::<ServiceRegistration>
        .into_iter()
        .map(|registration| (registration.constructor)())
        .collect()
}

pub fn registered_endpoints() -> Vec<EndpointDefinition> {
    inventory::iter::<EndpointRegistration>
        .into_iter()
        .map(|registration| (registration.constructor)())
        .collect()
}

pub fn registered_consumers() -> Vec<ConsumerDefinition> {
    inventory::iter::<ConsumerRegistration>
        .into_iter()
        .map(|registration| (registration.constructor)())
        .collect()
}
