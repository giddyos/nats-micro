/// Marks a request or response payload for end-to-end encryption.
#[derive(Debug, Clone)]
pub struct Encrypted<T>(pub T);

impl<T> Encrypted<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Encrypted<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Encrypted<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
