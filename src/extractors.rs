use std::{fmt::Display, str::FromStr, sync::Arc};

use bytes::Bytes;
use prost::Message;
use serde::de::DeserializeOwned;

use crate::{
    auth::Auth, error::NatsErrorResponse, handler::RequestContext, request::NatsRequest,
    utils::extract_subject_param,
};

pub trait FromRequest: Sized + Send + 'static {
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse>;
}

pub trait FromSubjectParam: Sized + Send + 'static {
    type Err: Display + Send;

    fn from_subject_param(value: &str) -> Result<Self, Self::Err>;
}

pub trait FromPayload: Sized + Send + 'static {
    fn from_payload(payload: &[u8], request_id: &str) -> Result<Self, NatsErrorResponse>;
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
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        T::from_payload(&ctx.request.payload, &ctx.request.request_id).map(Payload)
    }
}

#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

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

impl<T> FromRequest for Proto<T>
where
    T: Message + Default + Send + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
        T::decode(ctx.request.payload.clone())
            .map(Proto)
            .map_err(|e| {
                NatsErrorResponse::bad_request("BAD_PROTOBUF", e.to_string())
                    .with_request_id(ctx.request.request_id.clone())
            })
    }
}

impl<T> FromPayload for Json<T>
where
    T: DeserializeOwned + Send + 'static,
{
    fn from_payload(payload: &[u8], request_id: &str) -> Result<Self, NatsErrorResponse> {
        let value = serde_json::from_slice::<T>(payload).map_err(|e| {
            NatsErrorResponse::bad_request("BAD_JSON", e.to_string())
                .with_request_id(request_id.to_string())
        })?;
        Ok(Json(value))
    }
}

impl<T> FromPayload for Proto<T>
where
    T: Message + Default + Send + 'static,
{
    fn from_payload(payload: &[u8], request_id: &str) -> Result<Self, NatsErrorResponse> {
        T::decode(payload).map(Proto).map_err(|e| {
            NatsErrorResponse::bad_request("BAD_PROTOBUF", e.to_string())
                .with_request_id(request_id.to_string())
        })
    }
}

impl FromPayload for Bytes {
    fn from_payload(payload: &[u8], _request_id: &str) -> Result<Self, NatsErrorResponse> {
        Ok(Bytes::copy_from_slice(payload))
    }
}

impl FromPayload for Vec<u8> {
    fn from_payload(payload: &[u8], _request_id: &str) -> Result<Self, NatsErrorResponse> {
        Ok(payload.to_vec())
    }
}

impl FromPayload for String {
    fn from_payload(payload: &[u8], request_id: &str) -> Result<Self, NatsErrorResponse> {
        String::from_utf8(payload.to_vec()).map_err(|e| {
            NatsErrorResponse::bad_request("BAD_UTF8", e.to_string())
                .with_request_id(request_id.to_string())
        })
    }
}

impl<T> FromRequest for State<T>
where
    T: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, NatsErrorResponse> {
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

#[cfg(test)]
mod tests {
    use super::*;

    use async_nats::HeaderMap;
    use bytes::Bytes;

    use crate::{handler::RequestContext, request::NatsRequest, state::StateMap};

    #[derive(Clone, PartialEq, Message)]
    struct ExampleProto {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(uint32, tag = "2")]
        count: u32,
    }

    fn request_context(payload: Vec<u8>) -> RequestContext {
        RequestContext {
            request: NatsRequest {
                subject: "proto.example".to_string(),
                payload: Bytes::from(payload),
                headers: HeaderMap::new(),
                reply: None,
                request_id: "req-proto-1".to_string(),
            },
            states: StateMap::new(),
            user: None,
            subject_template: None,
            current_param_name: None,
        }
    }

    fn subject_param_context(subject: &str, template: &str, param_name: &str) -> RequestContext {
        RequestContext {
            request: NatsRequest {
                subject: subject.to_string(),
                payload: Bytes::new(),
                headers: HeaderMap::new(),
                reply: None,
                request_id: "req-subject-1".to_string(),
            },
            states: StateMap::new(),
            user: None,
            subject_template: Some(template.to_string()),
            current_param_name: Some(param_name.to_string()),
        }
    }

    #[derive(Debug, PartialEq)]
    struct FlexibleBool(bool);

    impl FromSubjectParam for FlexibleBool {
        type Err = &'static str;

        fn from_subject_param(value: &str) -> Result<Self, Self::Err> {
            match value {
                "1" | "true" => Ok(Self(true)),
                "0" | "false" => Ok(Self(false)),
                _ => Err("expected 1, 0, true, or false"),
            }
        }
    }

    #[test]
    fn prost_messages_decode_before_handler_execution() {
        let expected = ExampleProto {
            name: "created".to_string(),
            count: 7,
        };
        let ctx = request_context(expected.encode_to_vec());

        let decoded = Proto::<ExampleProto>::from_request(&ctx).expect("protobuf payload decodes");

        assert_eq!(decoded.0, expected);
    }

    #[test]
    fn invalid_protobuf_payloads_return_bad_request() {
        let ctx = request_context(vec![0xff, 0xff, 0xff]);

        let error =
            Proto::<ExampleProto>::from_request(&ctx).expect_err("invalid payload should fail");

        assert_eq!(error.code, 400);
        assert_eq!(error.error, "BAD_PROTOBUF");
        assert_eq!(error.request_id, "req-proto-1");
    }

    #[test]
    fn subject_params_use_default_integer_parsers() {
        let ctx = subject_param_context("users.42.profile", "users.{user_id}.profile", "user_id");

        let parsed = SubjectParam::<u32>::from_request(&ctx).expect("integer subject param parses");

        assert_eq!(parsed.0, 42);
    }

    #[test]
    fn subject_params_support_custom_parsers() {
        let ctx = subject_param_context("flags.1", "flags.{enabled}", "enabled");

        let parsed = SubjectParam::<FlexibleBool>::from_request(&ctx)
            .expect("custom subject param parser should be used");

        assert_eq!(parsed.0, FlexibleBool(true));
    }
}
