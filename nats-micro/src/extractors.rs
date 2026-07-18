#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

impl<T> Json<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct Proto<T>(pub T);

impl<T> Proto<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

macro_rules! impl_wrapper {
    ($wrapper:ident) => {
        impl<T> std::ops::Deref for $wrapper<T> {
            type Target = T;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T> std::ops::DerefMut for $wrapper<T> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

impl_wrapper!(Json);
impl_wrapper!(Proto);
