use std::{fmt::Display, future::Future, str::FromStr, sync::Arc};

use bytes::Bytes;
use nats_micro_shared::FrameworkError;

use crate::{
    auth::{Auth, AuthError, FromAuthRequest},
    error::{IntoNatsError, NatsErrorResponse},
    handler::RequestContext,
    prost::Message,
    request::{Headers, NatsRequest},
    serde::de::DeserializeOwned,
    shutdown_signal::ShutdownSignal,
    utils::extract_subject_param,
};

pub trait FromRequest: Sized + Send + 'static {
    fn from_request(
        ctx: &RequestContext,
    ) -> impl Future<Output = Result<Self, NatsErrorResponse>> + Send;
}

pub trait FromSubjectParam: Sized + Send + 'static {
    type Err: Display + Send;

    /// Parses a typed value from a subject placeholder segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the raw subject segment cannot be converted into
    /// the target type.
    fn from_subject_param(value: &str) -> Result<Self, Self::Err>;
}

pub trait FromPayload: Sized + Send + 'static {
    /// Extracts and decodes a handler payload from the request context.
    ///
    /// # Errors
    ///
    /// Returns an error when the request payload cannot be decoded into the
    /// target type.
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse>;
}

#[derive(Debug, Clone)]
pub struct Payload<T>(pub T);

impl<T> Payload<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &T {
        &self.0
    }

    #[must_use]
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> std::ops::Deref for Payload<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Payload<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: FromPayload> FromRequest for Payload<T> {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        T::from_payload(ctx).map(Payload)
    }
}

#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

impl<T> Json<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &T {
        &self.0
    }

    #[must_use]
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> std::ops::Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub struct Proto<T>(pub T);

impl<T> Proto<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &T {
        &self.0
    }

    #[must_use]
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct State<T>(pub Arc<T>);

impl<T> State<T> {
    #[must_use]
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::Deref for State<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::Deref for Proto<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Proto<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub struct SubjectParam<T>(pub T);

impl<T> SubjectParam<T> {
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &T {
        &self.0
    }

    #[must_use]
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> std::ops::Deref for SubjectParam<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for SubjectParam<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl RequestId {
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for RequestId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Subject(pub String);

impl Subject {
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }

    #[must_use]
    pub fn as_inner(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for Subject {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRequest for NatsRequest {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(ctx.request.clone())
    }
}

impl FromRequest for Headers {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(ctx.request.headers.clone())
    }
}

impl FromRequest for RequestId {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(RequestId(ctx.request.request_id.clone()))
    }
}

impl FromRequest for Subject {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(Subject(ctx.request.subject.clone()))
    }
}

impl FromRequest for ShutdownSignal {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        ctx.shutdown_signal().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::ShutdownSignalUnavailable,
                "This handler requested ShutdownSignal, but it was not registered with shutdown support.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })
    }
}

impl<T> FromPayload for Option<T>
where
    T: FromPayload,
{
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        if ctx.request.payload.is_empty() {
            return Ok(None);
        }

        match T::from_payload(ctx) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        }
    }
}

impl<T> FromPayload for Json<T>
where
    T: DeserializeOwned + Send + 'static,
{
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let value = crate::serde_json::from_slice::<T>(&ctx.request.payload).map_err(|e| {
            NatsErrorResponse::framework(
                FrameworkError::BadJson,
                format!("failed to decode the request payload as JSON: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;
        Ok(Json(value))
    }
}

impl<T> FromPayload for Proto<T>
where
    T: Message + Default + Send + 'static,
{
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        T::decode(ctx.request.payload.clone())
            .map(Proto)
            .map_err(|e| {
                NatsErrorResponse::framework(
                    FrameworkError::BadProtobuf,
                    format!("failed to decode the request payload as protobuf: {e}"),
                )
                .with_request_id(ctx.request.request_id.clone())
            })
    }
}

impl FromPayload for Bytes {
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(Bytes::copy_from_slice(&ctx.request.payload))
    }
}

impl FromPayload for Vec<u8> {
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(ctx.request.payload.to_vec())
    }
}

impl FromPayload for String {
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        String::from_utf8(ctx.request.payload.to_vec()).map_err(|e| {
            NatsErrorResponse::framework(
                FrameworkError::BadUtf8,
                format!("failed to decode the request payload as UTF-8 text: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })
    }
}

impl<T> FromRequest for State<T>
where
    T: Send + Sync + 'static,
{
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        ctx.states.get::<T>().map(State).ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::StateNotFound,
                format!(
                    "State `{}` was not registered in this NatsApp instance.",
                    std::any::type_name::<T>()
                ),
            )
            .with_request_id(ctx.request.request_id.clone())
        })
    }
}

impl<U> FromRequest for Auth<U>
where
    U: FromAuthRequest,
{
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        U::from_auth_request(ctx)
            .await
            .map(Auth::new)
            .map_err(|error| error.into_nats_error(ctx.request.request_id.clone()))
    }
}

impl<U> FromRequest for Option<Auth<U>>
where
    U: FromAuthRequest,
{
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        match U::from_auth_request(ctx).await {
            Ok(user) => Ok(Some(Auth::new(user))),
            Err(AuthError::MissingCredentials) => Ok(None),
            Err(error) => Err(error.into_nats_error(ctx.request.request_id.clone())),
        }
    }
}

impl FromSubjectParam for String {
    type Err = std::convert::Infallible;

    fn from_subject_param(value: &str) -> Result<Self, Self::Err> {
        Ok(value.to_string())
    }
}

macro_rules! impl_from_subject_param_via_from_str {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromSubjectParam for $ty {
                type Err = <$ty as FromStr>::Err;

                fn from_subject_param(value: &str) -> Result<Self, Self::Err> {
                    value.parse::<$ty>()
                }
            }
        )*
    };
}

impl_from_subject_param_via_from_str!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

impl<T> FromRequest for SubjectParam<T>
where
    T: FromSubjectParam,
{
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let param_name = ctx.current_param_name.as_deref().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::ParamNameMissing,
                "Subject parameter extraction requires macro-generated parameter metadata, but none was provided.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let template = ctx.subject_template.as_deref().ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::SubjectTemplateMissing,
                "Subject parameter extraction requires a macro-generated subject template, but none was provided.",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let raw =
            extract_subject_param(template, &ctx.request.subject, param_name).ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::SubjectParamMissing,
                    format!(
                        "Subject parameter `{param_name}` was not found in subject `{}`.",
                        ctx.request.subject
                    ),
                )
                .with_request_id(ctx.request.request_id.clone())
            })?;

        let parsed = T::from_subject_param(&raw).map_err(|e| {
            NatsErrorResponse::framework(
                FrameworkError::SubjectParamInvalid,
                format!("failed to parse subject parameter `{param_name}` from value `{raw}`: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        Ok(SubjectParam(parsed))
    }
}
