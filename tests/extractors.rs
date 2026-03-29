use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro::{
    FromPayload, FromRequest, FromSubjectParam, NatsRequest, Proto, RequestContext, StateMap,
    Subject, SubjectParam,
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
            headers: HeaderMap::new(),
            reply: None,
            request_id: "req-proto-1".to_string(),
        },
        StateMap::new(),
        None,
        None,
    )
}

fn subject_param_context(subject: &str, template: &str, param_name: &str) -> RequestContext {
    let mut ctx = RequestContext::new(
        NatsRequest {
            subject: subject.to_string(),
            payload: Bytes::new(),
            headers: HeaderMap::new(),
            reply: None,
            request_id: "req-subject-1".to_string(),
        },
        StateMap::new(),
        None,
        Some(template.to_string()),
    );
    ctx.current_param_name = Some(param_name.to_string());
    ctx
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

    let decoded = Proto::<ExampleProto>::from_payload(&ctx).expect("protobuf payload decodes");

    assert_eq!(decoded.0, expected);
}

#[test]
fn invalid_protobuf_payloads_return_bad_request() {
    let ctx = request_context(vec![0xff, 0xff, 0xff]);

    let error = Proto::<ExampleProto>::from_payload(&ctx).expect_err("invalid payload should fail");

    assert_eq!(error.code, 400);
    assert_eq!(error.error, "BAD_PROTOBUF");
    assert_eq!(error.request_id, "req-proto-1");
    assert!(!error.message.is_empty());
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

#[test]
fn missing_subject_params_return_bad_request() {
    let ctx = subject_param_context("users.profile", "users.{user_id}.profile", "user_id");

    let error =
        SubjectParam::<u32>::from_request(&ctx).expect_err("missing subject param should fail");

    assert_eq!(error.code, 400);
    assert_eq!(error.error, "SUBJECT_PARAM_MISSING");
    assert_eq!(error.request_id, "req-subject-1");
    assert!(!error.message.is_empty());
}

#[test]
fn subject_extractor_works() {
    let ctx = request_context(vec![]);
    let subject = Subject::from_request(&ctx).expect("subject extractor should work");
    assert_eq!(subject.0, "proto.example");
}
