use std::{fmt::Display, future::Future, str::FromStr, sync::Arc};

use bytes::Bytes;

use crate::{
    auth::{Auth, AuthError, FromAuthRequest},
    error::{IntoNatsError, NatsErrorResponse},
    handler::RequestContext,
    prost::Message,
    request::{Header, Headers, NatsRequest},
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

    fn from_subject_param(value: &str) -> Result<Self, Self::Err>;
}

pub trait FromPayload: Sized + Send + 'static {
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse>;
}

#[derive(Debug, Clone)]
pub struct Payload<T>(pub T);

impl<T> std::ops::Deref for Payload<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: FromPayload> FromRequest for Payload<T> {
    async fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        T::from_payload(ctx).map(Payload)
    }
}

#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

impl<T> std::ops::Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Proto<T>(pub T);

#[derive(Clone)]
pub struct State<T>(pub Arc<T>);

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

#[derive(Debug, Clone)]
pub struct SubjectParam<T>(pub T);

impl<T> std::ops::Deref for SubjectParam<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl std::ops::Deref for RequestId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Subject(pub String);

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
            NatsErrorResponse::internal(
                "SHUTDOWN_SIGNAL_UNAVAILABLE",
                "shutdown signal extractor was requested but this handler was not registered with shutdown support",
            )
            .with_request_id(ctx.request.request_id.clone())
        })
    }
}

impl<T> FromPayload for Json<T>
where
    T: DeserializeOwned + Send + 'static,
{
    fn from_payload(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let value = crate::serde_json::from_slice::<T>(&ctx.request.payload).map_err(|e| {
            NatsErrorResponse::bad_request("BAD_JSON", e.to_string())
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
                NatsErrorResponse::bad_request("BAD_PROTOBUF", e.to_string())
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
            NatsErrorResponse::bad_request("BAD_UTF8", e.to_string())
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
            NatsErrorResponse::internal(
                "STATE_NOT_FOUND",
                format!("state `{}` was not registered", std::any::type_name::<T>()),
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
            NatsErrorResponse::internal(
                "PARAM_NAME_MISSING",
                "subject param extraction requires macro-generated param metadata",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let template = ctx.subject_template.as_deref().ok_or_else(|| {
            NatsErrorResponse::internal(
                "SUBJECT_TEMPLATE_MISSING",
                "subject param extraction requires macro-generated subject template",
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        let raw =
            extract_subject_param(template, &ctx.request.subject, param_name).ok_or_else(|| {
                NatsErrorResponse::bad_request(
                    "SUBJECT_PARAM_MISSING",
                    format!("subject parameter `{param_name}` was not present"),
                )
                .with_request_id(ctx.request.request_id.clone())
            })?;

        let parsed = T::from_subject_param(&raw).map_err(|e| {
            NatsErrorResponse::bad_request(
                "SUBJECT_PARAM_INVALID",
                format!("failed to parse `{param_name}`: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        Ok(SubjectParam(parsed))
    }
}
