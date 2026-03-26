use once_cell::sync::Lazy;
use parking_lot::RwLock;

use crate::{
    consumer::ConsumerDefinition,
    service::{EndpointDefinition, ServiceMetadata},
};

static SERVICES: Lazy<RwLock<Vec<ServiceMetadata>>> = Lazy::new(|| RwLock::new(Vec::new()));
static ENDPOINTS: Lazy<RwLock<Vec<EndpointDefinition>>> = Lazy::new(|| RwLock::new(Vec::new()));
static CONSUMERS: Lazy<RwLock<Vec<ConsumerDefinition>>> = Lazy::new(|| RwLock::new(Vec::new()));

pub fn register_service(service: ServiceMetadata) {
    SERVICES.write().push(service);
}

pub fn register_endpoint(endpoint: EndpointDefinition) {
    ENDPOINTS.write().push(endpoint);
}

pub fn register_consumer(consumer: ConsumerDefinition) {
    CONSUMERS.write().push(consumer);
}

pub fn registered_services() -> Vec<ServiceMetadata> {
    SERVICES.read().clone()
}

pub fn registered_endpoints() -> Vec<EndpointDefinition> {
    ENDPOINTS.read().clone()
}

pub fn registered_consumers() -> Vec<ConsumerDefinition> {
    CONSUMERS.read().clone()
}
