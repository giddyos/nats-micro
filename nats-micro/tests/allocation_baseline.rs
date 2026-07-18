use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use nats_micro::{
    __macros::into_handler_fn, AuthPolicy, Bytes, Codec, DispatchResult, NatsErrorResponse,
    NatsRequest, OperationKind, OperationSpec, OwnedHeaders, Request, RequestContext,
    RequestEndpoint, Response, StateMap,
};

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

async fn static_response() -> Result<&'static str, NatsErrorResponse> {
    Ok("ok")
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
fn v1_dynamic_raw_static_response_allocation_baseline() {
    let state = StateMap::new();
    let handler = into_handler_fn::<_, ()>(static_response);

    let (response, allocations) = count_allocations(|| {
        let context = RequestContext::new(
            NatsRequest {
                subject: "users.v1.users.lookup".to_owned(),
                payload: Bytes::from_static(b"request"),
                headers: OwnedHeaders::new(),
                reply: Some("_INBOX.baseline".to_owned()),
                request_id: "019b-baseline-request".to_owned(),
            },
            state.clone(),
            Some("users.v1.users.lookup".to_owned()),
        );
        let mut future = handler.call(context);
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(response) => response.expect("static response"),
            Poll::Pending => panic!("baseline handler unexpectedly yielded"),
        }
    });

    assert_eq!(response.payload, Bytes::from_static(b"ok"));
    assert_eq!(
        allocations, 6,
        "update the recorded v1 baseline when the dynamic path changes"
    );
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
fn static_worker_sources_exclude_dynamic_hot_path_primitives() {
    let request_worker = include_str!("../src/runtime/request_worker.rs");
    let consumer_worker = include_str!("../src/runtime/consumer_worker.rs");
    let sources = [request_worker, consumer_worker];

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
}
