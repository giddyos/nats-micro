use std::future::Future;

/// Empty type-level service list.
#[derive(Debug, Clone, Copy, Default)]
pub struct Nil;

/// One element in a type-level service list.
#[derive(Debug, Clone, Copy)]
pub struct Cons<Head, Tail> {
    pub head: Head,
    pub tail: Tail,
}

/// A generated service that starts only concrete static workers.
pub trait Service<S>: Copy + Send + Sync + 'static {
    const SPEC: crate::ServiceSpec;

    type Client<T>: Clone
    where
        T: crate::ClientTransport;

    fn client<T>(transport: T) -> Self::Client<T>
    where
        T: crate::ClientTransport;

    fn start(
        self,
        runtime: &mut crate::Runtime<S>,
    ) -> impl Future<Output = Result<(), crate::StartError>> + Send;

    #[cfg(feature = "test-util")]
    fn dispatch_local(
        self,
        state: &S,
        request: crate::testing::LocalRequest,
    ) -> impl Future<Output = crate::testing::LocalDispatch> + Send + '_;

    #[cfg(feature = "test-jetstream")]
    fn dispatch_consumer_local<'a>(
        self,
        state: &'a S,
        durable: &'a str,
        request: crate::testing::LocalRequest,
    ) -> impl Future<Output = crate::testing::LocalConsumerDispatch> + Send + 'a;
}

/// Recursive startup and validation for a type-level service list.
pub trait ServiceSet<S>: Send + Sync + 'static {
    fn start_all(
        self,
        runtime: &mut crate::Runtime<S>,
    ) -> impl Future<Output = Result<(), crate::StartError>> + Send;

    fn validate_all(&self, validator: &mut ServiceSetValidator) -> Result<(), crate::StartError>;
}

impl<S> ServiceSet<S> for Nil
where
    S: Send + Sync + 'static,
{
    async fn start_all(self, _runtime: &mut crate::Runtime<S>) -> Result<(), crate::StartError> {
        Ok(())
    }

    fn validate_all(&self, _validator: &mut ServiceSetValidator) -> Result<(), crate::StartError> {
        Ok(())
    }
}

impl<S, Head, Tail> ServiceSet<S> for Cons<Head, Tail>
where
    S: Send + Sync + 'static,
    Head: Service<S>,
    Tail: ServiceSet<S>,
{
    async fn start_all(self, runtime: &mut crate::Runtime<S>) -> Result<(), crate::StartError> {
        self.head.start(runtime).await?;
        self.tail.start_all(runtime).await
    }

    fn validate_all(&self, validator: &mut ServiceSetValidator) -> Result<(), crate::StartError> {
        validator.add(&Head::SPEC)?;
        self.tail.validate_all(validator)
    }
}

/// Deterministic startup validation for fluent applications.
#[derive(Default)]
pub struct ServiceSetValidator {
    operations: std::collections::BTreeMap<&'static str, (&'static str, &'static str)>,
    consumers:
        std::collections::BTreeMap<(&'static str, &'static str), (&'static str, &'static str)>,
}

impl ServiceSetValidator {
    fn add(&mut self, service: &'static crate::ServiceSpec) -> Result<(), crate::StartError> {
        for operation in service.operations {
            if let Some((other_service, other_operation)) = self
                .operations
                .insert(operation.subject, (service.name, operation.rust_name))
            {
                anyhow::bail!(
                    "duplicate subject `{}`: `{}::{}` conflicts with `{}::{}`",
                    operation.subject,
                    other_service,
                    other_operation,
                    service.name,
                    operation.rust_name,
                );
            }
        }
        for consumer in service.consumers {
            let key = (consumer.stream, consumer.durable);
            if let Some((other_service, other_consumer)) = self
                .consumers
                .insert(key, (service.name, consumer.rust_name))
            {
                anyhow::bail!(
                    "duplicate consumer `{}/{}`: `{}::{}` conflicts with `{}::{}`",
                    consumer.stream,
                    consumer.durable,
                    other_service,
                    other_consumer,
                    service.name,
                    consumer.rust_name,
                );
            }
        }
        Ok(())
    }
}

pub fn validate_service_set<S, Services>(services: &Services) -> Result<(), crate::StartError>
where
    S: Send + Sync + 'static,
    Services: ServiceSet<S>,
{
    services.validate_all(&mut ServiceSetValidator::default())
}

#[must_use]
pub const fn str_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

pub const fn assert_services_compatible(left: &crate::ServiceSpec, right: &crate::ServiceSpec) {
    let mut left_index = 0;
    while left_index < left.operations.len() {
        let mut right_index = 0;
        while right_index < right.operations.len() {
            assert!(
                !str_eq(
                    left.operations[left_index].subject,
                    right.operations[right_index].subject,
                ),
                "duplicate request/publish/subscribe subject in application"
            );
            right_index += 1;
        }
        left_index += 1;
    }

    let mut left_consumer = 0;
    while left_consumer < left.consumers.len() {
        let mut right_consumer = 0;
        while right_consumer < right.consumers.len() {
            let same_stream = str_eq(
                left.consumers[left_consumer].stream,
                right.consumers[right_consumer].stream,
            );
            let same_durable = str_eq(
                left.consumers[left_consumer].durable,
                right.consumers[right_consumer].durable,
            );
            assert!(
                !(same_stream && same_durable),
                "duplicate JetStream consumer in application"
            );
            right_consumer += 1;
        }
        left_consumer += 1;
    }
}
