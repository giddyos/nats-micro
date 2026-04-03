use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro::{
    Auth, AuthError, FromAuthRequest, FromPayload, FromRequest, FromSubjectParam, Headers,
    NatsRequest, Proto, RequestContext, ShutdownSignal, StateMap, Subject, SubjectParam,
};
use prost::Message;

#[derive(Clone, PartialEq, Message)]
struct ExampleProto {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(uint32, tag = "2")]
    count: u32,
}

fn request_context(payload: Vec<u8>) -> RequestContext {
    RequestContext::new(
        NatsRequest {
            subject: "proto.example".to_string(),
            payload: Bytes::from(payload),
            headers: HeaderMap::new().into(),
            reply: None,
            request_id: "req-proto-1".to_string(),
        },
        StateMap::new(),
        None,
    )
}

fn subject_param_context(subject: &str, template: &str, param_name: &str) -> RequestContext {
    let mut ctx = RequestContext::new(
        NatsRequest {
            subject: subject.to_string(),
            payload: Bytes::new(),
            headers: HeaderMap::new().into(),
            reply: None,
            request_id: "req-subject-1".to_string(),
        },
        StateMap::new(),
        Some(template.to_string()),
    );
    ctx.current_param_name = Some(param_name.to_string());
    ctx
}

#[derive(Debug, PartialEq)]
struct FlexibleBool(bool);

#[derive(Debug, PartialEq, Eq)]
struct JwtUser {
    subject: String,
}

impl FromAuthRequest for JwtUser {
    async fn from_auth_request(ctx: &RequestContext) -> Result<Self, AuthError> {
        match ctx
            .request
            .headers
            .get("authorization")
            .map(|value| value.as_str())
        {
            Some("Bearer demo-token") => Ok(Self {
                subject: "demo-user".to_string(),
            }),
            Some(_) => Err(AuthError::InvalidCredentials),
            None => Err(AuthError::MissingCredentials),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ApiUser {
    key_id: String,
}

impl FromAuthRequest for ApiUser {
    async fn from_auth_request(ctx: &RequestContext) -> Result<Self, AuthError> {
        match ctx
            .request
            .headers
            .get("x-api-key")
            .map(|value| value.as_str())
        {
            Some("demo-key") => Ok(Self {
                key_id: "api-demo".to_string(),
            }),
            Some(_) => Err(AuthError::Forbidden),
            None => Err(AuthError::MissingCredentials),
        }
    }
}

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

    let decoded = Proto::<ExampleProto>::from_payload(&ctx).expect("protobuf payload decodes");

    assert_eq!(decoded.0, expected);
}

#[test]
fn invalid_protobuf_payloads_return_bad_request() {
    let ctx = request_context(vec![0xff, 0xff, 0xff]);

    let error = Proto::<ExampleProto>::from_payload(&ctx).expect_err("invalid payload should fail");

    assert_eq!(error.code, 400);
    assert_eq!(error.kind, "BAD_PROTOBUF");
    assert_eq!(error.request_id, "req-proto-1");
    assert!(!error.message.is_empty());
}

#[tokio::test]
async fn subject_params_use_default_integer_parsers() {
    let ctx = subject_param_context("users.42.profile", "users.{user_id}.profile", "user_id");

    let parsed = SubjectParam::<u32>::from_request(&ctx)
        .await
        .expect("integer subject param parses");

    assert_eq!(parsed.0, 42);
}

#[tokio::test]
async fn subject_params_support_custom_parsers() {
    let ctx = subject_param_context("flags.1", "flags.{enabled}", "enabled");

    let parsed = SubjectParam::<FlexibleBool>::from_request(&ctx)
        .await
        .expect("custom subject param parser should be used");

    assert_eq!(parsed.0, FlexibleBool(true));
}

#[tokio::test]
async fn missing_subject_params_return_bad_request() {
    let ctx = subject_param_context("users.profile", "users.{user_id}.profile", "user_id");

    let error = SubjectParam::<u32>::from_request(&ctx)
        .await
        .expect_err("missing subject param should fail");

    assert_eq!(error.code, 400);
    assert_eq!(error.kind, "SUBJECT_PARAM_MISSING");
    assert_eq!(error.request_id, "req-subject-1");
    assert!(!error.message.is_empty());
}

#[tokio::test]
async fn subject_extractor_works() {
    let ctx = request_context(vec![]);
    let subject = Subject::from_request(&ctx)
        .await
        .expect("subject extractor should work");
    assert_eq!(subject.0, "proto.example");
}

#[tokio::test]
async fn shutdown_signal_extractor_requires_runtime_wiring() {
    let ctx = request_context(vec![]);

    let err = match ShutdownSignal::from_request(&ctx).await {
        Ok(_) => panic!("shutdown extractor should fail without runtime wiring"),
        Err(err) => err,
    };

    assert_eq!(err.code, 500);
    assert_eq!(err.kind, "SHUTDOWN_SIGNAL_UNAVAILABLE");
    assert_eq!(err.request_id, "req-proto-1");
}

#[tokio::test]
async fn headers_extractor_returns_plaintext_headers() {
    let mut ctx = request_context(vec![]);
    ctx.request.headers.insert("x-request-id", "req-proto-1");
    ctx.request.headers.append("x-role", "admin");
    ctx.request.headers.append("x-role", "writer");

    let headers = Headers::from_request(&ctx)
        .await
        .expect("headers extractor should work");

    assert_eq!(headers.len(), 3);
    assert_eq!(headers[0].key, "x-request-id");
    assert_eq!(headers[0].value, "req-proto-1");
    assert_eq!(headers[1].key, "x-role");
    assert_eq!(headers[1].value, "admin");
    assert_eq!(headers[2].key, "x-role");
    assert_eq!(headers[2].value, "writer");
}

#[tokio::test]
async fn auth_extractors_resolve_each_type_from_the_request() {
    let mut ctx = request_context(vec![]);
    ctx.request
        .headers
        .insert("authorization", "Bearer demo-token");
    ctx.request.headers.insert("x-api-key", "demo-key");

    let jwt_user = Auth::<JwtUser>::from_request(&ctx)
        .await
        .expect("jwt auth resolves");
    let api_user = Auth::<ApiUser>::from_request(&ctx)
        .await
        .expect("api-key auth resolves");

    assert_eq!(jwt_user.subject, "demo-user");
    assert_eq!(api_user.key_id, "api-demo");
}

#[tokio::test]
async fn optional_auth_returns_none_when_credentials_are_missing() {
    let ctx = request_context(vec![]);

    let user = Option::<Auth<JwtUser>>::from_request(&ctx)
        .await
        .expect("missing credentials stay optional");

    assert!(user.is_none());
}

#[tokio::test]
async fn optional_auth_preserves_non_missing_auth_failures() {
    let mut ctx = request_context(vec![]);
    ctx.request.headers.insert("x-api-key", "wrong-key");

    let error = match Option::<Auth<ApiUser>>::from_request(&ctx).await {
        Ok(_) => panic!("invalid credentials should still fail"),
        Err(error) => error,
    };

    assert_eq!(error.code, 403);
    assert_eq!(error.kind, "FORBIDDEN");
    assert_eq!(error.request_id, "req-proto-1");
}
