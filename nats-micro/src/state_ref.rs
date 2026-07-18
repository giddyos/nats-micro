use std::ops::Deref;

/// A compile-time projection from one application state into a field type.
pub trait FromAppState<S>: Sized {
    fn from_state(state: &S) -> &Self;
}

/// A borrowed, statically resolved application-state field.
#[derive(Debug, Clone, Copy)]
pub struct StateRef<'a, T: ?Sized>(&'a T);

impl<'a, T: ?Sized> StateRef<'a, T> {
    #[inline]
    #[must_use]
    pub const fn new(value: &'a T) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> &'a T {
        self.0
    }
}

impl<T: ?Sized> Deref for StateRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
