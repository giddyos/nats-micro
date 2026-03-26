use std::{str::FromStr, sync::Arc};

use serde::de::DeserializeOwned;

use crate::{
    auth::Auth,
    error::NatsErrorResponse,
    handler::RequestContext,
    request::NatsRequest,
    utils::extract_subject_param,
};

pub trait FromRequest: Sized + Send + 'static {
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse>;
}

#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

#[derive(Clone)]
pub struct State<T>(pub Arc<T>);

impl<T> std::ops::Deref for State<T> {
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

impl FromRequest for NatsRequest {
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(ctx.request.clone())
    }
}

impl FromRequest for RequestId {
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        Ok(RequestId(ctx.request.request_id.clone()))
    }
}

impl<T> FromRequest for Json<T>
where
    T: DeserializeOwned + Send + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let value = serde_json::from_slice::<T>(&ctx.request.payload).map_err(|e| {
            NatsErrorResponse::bad_request("BAD_JSON", e.to_string())
                .with_request_id(ctx.request.request_id.clone())
        })?;
        Ok(Json(value))
    }
}

impl<T> FromRequest for State<T>
where
    T: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        ctx.states
            .get::<T>()
            .map(State)
            .ok_or_else(|| {
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
    U: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        let user = ctx.user.as_ref().ok_or_else(|| {
            NatsErrorResponse::unauthorized("UNAUTHORIZED", "authentication required")
                .with_request_id(ctx.request.request_id.clone())
        })?;

        let typed = user.clone().downcast::<U>().map_err(|_| {
            NatsErrorResponse::internal(
                "AUTH_TYPE_MISMATCH",
                format!(
                    "requested auth type `{}` did not match resolver output",
                    std::any::type_name::<U>()
                ),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        Ok(Auth::new(typed))
    }
}

impl<U> FromRequest for Option<Auth<U>>
where
    U: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        match ctx.user.as_ref() {
            Some(user) => {
                let typed = user.clone().downcast::<U>().map_err(|_| {
                    NatsErrorResponse::internal(
                        "AUTH_TYPE_MISMATCH",
                        format!(
                            "requested auth type `{}` did not match resolver output",
                            std::any::type_name::<U>()
                        ),
                    )
                    .with_request_id(ctx.request.request_id.clone())
                })?;
                Ok(Some(Auth::new(typed)))
            }
            None => Ok(None),
        }
    }
}

impl<T> FromRequest for SubjectParam<T>
where
    T: FromStr + Send + 'static,
    <T as FromStr>::Err: std::fmt::Display + Send,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
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

        let raw = extract_subject_param(template, &ctx.request.subject, param_name).ok_or_else(
            || {
                NatsErrorResponse::bad_request(
                    "SUBJECT_PARAM_MISSING",
                    format!("subject parameter `{param_name}` was not present"),
                )
                .with_request_id(ctx.request.request_id.clone())
            },
        )?;

        let parsed = raw.parse::<T>().map_err(|e| {
            NatsErrorResponse::bad_request(
                "SUBJECT_PARAM_INVALID",
                format!("failed to parse `{param_name}`: {e}"),
            )
            .with_request_id(ctx.request.request_id.clone())
        })?;

        Ok(SubjectParam(parsed))
    }
}
