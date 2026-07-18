use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use nats_micro::{
    AuthPolicy, Bytes, Codec, DispatchResult, OperationKind, OperationSpec, Request,
    RequestEndpoint, Response,
};
use serde::Serialize;

struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNTING.with(|counting| {
            if counting.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        COUNTING.with(|counting| {
            if counting.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNTING.with(|counting| {
            if counting.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.with(|allocations| allocations.set(0));
    COUNTING.with(|counting| counting.set(true));
    let output = f();
    COUNTING.with(|counting| counting.set(false));
    let allocations = ALLOCATIONS.with(Cell::get);
    (output, allocations)
}

struct StaticEndpoint;

impl RequestEndpoint<()> for StaticEndpoint {
    const SPEC: OperationSpec = OperationSpec {
        service_name: "allocation",
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

    async fn call<'a>((): &'a (), request: Request<'a>) -> DispatchResult {
        assert_eq!(request.subject(), Self::SPEC.subject);
        assert_eq!(request.body(), b"request");
        Ok(Response::bytes(Bytes::from_static(b"ok")))
    }
}

#[test]
fn raw_borrowed_static_response_has_zero_framework_allocations() {
    let state = ();

    let (response, allocations) = count_allocations(|| {
        let request = Request::new(
            "users.v2.users.lookup",
            Some("_INBOX.static"),
            b"request",
            None,
        );
        let future = StaticEndpoint::call(&state, request);
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(response) => response.expect("static response"),
            Poll::Pending => panic!("static endpoint unexpectedly yielded"),
        }
    });

    match response {
        Response::Payload(payload) => assert_eq!(payload, Bytes::from_static(b"ok")),
        other => panic!("unexpected response: {other:?}"),
    }
    assert_eq!(allocations, 0);
}

#[test]
fn static_subject_and_state_access_have_zero_framework_allocations() {
    struct State {
        value: u64,
    }
    let state = State { value: 42 };

    let ((subject, value), allocations) =
        count_allocations(|| (StaticEndpoint::SPEC.subject, state.value));

    assert_eq!(subject, "users.v2.users.lookup");
    assert_eq!(value, 42);
    assert_eq!(allocations, 0);
}

#[derive(Serialize)]
struct JsonOutput<'a> {
    name: &'a str,
}

#[test]
fn json_response_serialization_uses_one_output_allocation() {
    let _ = nats_micro::encode_json(&JsonOutput { name: "warmup" }).expect("warm JSON encoder");
    let (encoded, allocations) = count_allocations(|| {
        nats_micro::encode_json(&JsonOutput { name: "Ada" }).expect("encode JSON")
    });

    assert_eq!(encoded, br#"{"name":"Ada"}"#.as_slice());
    assert_eq!(allocations, 1);
}

#[cfg(feature = "protobuf")]
#[derive(Clone, PartialEq, nats_micro::prost::Message)]
struct ProtoOutput {
    #[prost(string, tag = "1")]
    name: String,
}

#[cfg(feature = "protobuf")]
#[test]
fn protobuf_response_encoding_allocates_once_without_growth() {
    let output = ProtoOutput {
        name: "Ada".to_owned(),
    };
    let encoded_len = nats_micro::prost::Message::encoded_len(&output);
    let _ = nats_micro::encode_proto(&output).expect("warm protobuf encoder");
    let (encoded, allocations) =
        count_allocations(|| nats_micro::encode_proto(&output).expect("encode protobuf"));

    assert_eq!(encoded.len(), encoded_len);
    assert_eq!(allocations, 1);
}

#[test]
fn static_worker_sources_exclude_dynamic_hot_path_primitives() {
    let request_worker = include_str!("../src/runtime/request_worker.rs");
    let consumer_worker = include_str!("../src/runtime/consumer_worker.rs");
    let subscription_worker = include_str!("../src/runtime/subscription_worker.rs");
    let sources = [request_worker, consumer_worker, subscription_worker];

    for source in sources {
        for prohibited in [
            "HandlerFn",
            "HandlerFuture",
            "RequestContext",
            "StateMap",
            "Semaphore",
            "acquire_owned",
            "JoinSet",
            "tokio::spawn",
            ".clone(",
            "unsafe ",
        ] {
            assert!(
                !source.contains(prohibited),
                "static worker contains prohibited `{prohibited}`"
            );
        }
        assert!(source.contains("FuturesUnordered"));
        assert!(source.contains("in_flight.len() < concurrency"));
    }

    let generated_dispatch = include_str!("../../nats-micro-macros/src/service/dispatch.rs");
    for prohibited in [
        "Box < dyn Future",
        "Box<dyn Future",
        "request.body().clone",
        "request.clone",
        "tokio::spawn",
        "StateMap",
    ] {
        assert!(
            !generated_dispatch.contains(prohibited),
            "generated dispatch contains prohibited `{prohibited}`"
        );
    }

    let state_projection = include_str!("../src/state_ref.rs");
    for prohibited in ["HashMap", "TypeId", "Any", "downcast", "Atomic", "Arc"] {
        assert!(
            !state_projection.contains(prohibited),
            "state projection contains prohibited `{prohibited}`"
        );
    }

    let primary_api = [
        include_str!("../src/lib.rs"),
        include_str!("../src/prelude.rs"),
        include_str!("../src/extractors.rs"),
        include_str!("../src/service.rs"),
    ]
    .concat();
    for prohibited in ["Payload<T>", "IntoPayloadInner", "build_subject"] {
        assert!(
            !primary_api.contains(prohibited),
            "primary v2 API still exposes removed `{prohibited}`"
        );
    }
}
