use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

#[derive(Default, Clone)]
pub struct StateMap {
    inner: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl StateMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T>(mut self, value: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        let mut map = (*self.inner).clone();
        map.insert(TypeId::of::<T>(), Arc::new(value));
        self.inner = Arc::new(map);
        self
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.inner
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|v| Arc::downcast::<T>(v).ok())
    }
}
