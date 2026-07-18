use criterion::{Criterion, criterion_group, criterion_main};
use nats_micro::{
    __macros::into_handler_fn, AuthPolicy, Bytes, Codec, DispatchResult, NatsErrorResponse,
    NatsRequest, OperationKind, OperationSpec, Request, RequestContext, RequestEndpoint, Response,
    StateMap,
};

const PAYLOAD: &[u8] = br#"{"name":"Ada"}"#;
const REPLY: &str = "_INBOX.baseline";
const REQUEST_ID: &str = "019b-benchmark-request";
const SUBJECT: &str = "users.v1.users.lookup";
const V2_SUBJECT: &str = "users.v2.users.lookup";

async fn static_response() -> Result<&'static str, NatsErrorResponse> {
    Ok("ok")
}

struct StaticEndpoint;

impl RequestEndpoint<()> for StaticEndpoint {
    const SPEC: OperationSpec = OperationSpec {
        service_name: "bench",
        service_version: "1.0.0",
        rust_name: "raw_borrowed_static_response",
        kind: OperationKind::Request,
        subject: "users.v2.users.lookup",
        subject_template: "users.v2.users.lookup",
        queue_group: None,
        request_codec: Codec::Raw,
        response_codec: Codec::Raw,
        request_encrypted: false,
        response_encrypted: false,
        request_type: Some("&[u8]"),
        response_type: Some("&'static [u8]"),
        error_type: None,
        auth: AuthPolicy::None,
        concurrency: 1,
        params: &[],
    };

    async fn call<'a>((): &'a (), _: Request<'a>) -> DispatchResult {
        Ok(Response::bytes(Bytes::from_static(b"ok")))
    }
}

fn request_context(state: &StateMap) -> RequestContext {
    RequestContext::new(
        NatsRequest {
            subject: SUBJECT.to_owned(),
            payload: Bytes::from_static(PAYLOAD),
            headers: nats_micro::Headers::new(),
            reply: Some(REPLY.to_owned()),
            request_id: REQUEST_ID.to_owned(),
        },
        state.clone(),
        Some(SUBJECT.to_owned()),
    )
}

fn request_dispatch(criterion: &mut Criterion) {
    let state = StateMap::new();
    let handler = into_handler_fn::<_, ()>(static_response);
    let runtime = tokio::runtime::Runtime::new().expect("benchmark runtime");

    criterion.bench_function("v1_dynamic_raw_static_response", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(handler.call(request_context(&state)))
                .expect("static response")
        });
    });

    criterion.bench_function("v2_static_raw_borrowed_response", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(StaticEndpoint::call(
                    &(),
                    Request::new(V2_SUBJECT, Some(REPLY), PAYLOAD, None),
                ))
                .expect("static response")
        });
    });
}

criterion_group!(benches, request_dispatch);
criterion_main!(benches);
