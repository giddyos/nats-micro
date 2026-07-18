/// Recursively removes the lightweight wire wrappers used in service method
/// signatures.
pub trait IntoPayloadInner: Sized {
    type Inner;

    fn into_payload_inner(self) -> Self::Inner;
}

#[derive(Debug, Clone)]
pub struct Payload<T>(pub T);

impl<T> Payload<T> {
    #[must_use]
    pub fn into_inner(self) -> <Self as IntoPayloadInner>::Inner
    where
        Self: IntoPayloadInner,
    {
        self.into_payload_inner()
    }

    #[must_use]
    pub fn into_wrapped(self) -> T {
        self.0
    }
}

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

impl_wrapper!(Payload);
impl_wrapper!(Json);
impl_wrapper!(Proto);

impl<T> IntoPayloadInner for Payload<T>
where
    T: IntoPayloadInner,
{
    type Inner = T::Inner;

    fn into_payload_inner(self) -> Self::Inner {
        self.0.into_payload_inner()
    }
}

impl<T> IntoPayloadInner for Json<T> {
    type Inner = T;

    fn into_payload_inner(self) -> Self::Inner {
        self.0
    }
}

impl<T> IntoPayloadInner for Proto<T> {
    type Inner = T;

    fn into_payload_inner(self) -> Self::Inner {
        self.0
    }
}

impl<T> IntoPayloadInner for Option<T>
where
    T: IntoPayloadInner,
{
    type Inner = Option<T::Inner>;

    fn into_payload_inner(self) -> Self::Inner {
        self.map(IntoPayloadInner::into_payload_inner)
    }
}

impl IntoPayloadInner for bytes::Bytes {
    type Inner = Self;

    fn into_payload_inner(self) -> Self::Inner {
        self
    }
}

impl IntoPayloadInner for Vec<u8> {
    type Inner = Self;

    fn into_payload_inner(self) -> Self::Inner {
        self
    }
}

impl IntoPayloadInner for String {
    type Inner = Self;

    fn into_payload_inner(self) -> Self::Inner {
        self
    }
}
