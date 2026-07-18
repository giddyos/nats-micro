use std::{hint::black_box, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use futures_util::StreamExt;
use nats_micro::{
    AuthPolicy, Bytes, Codec, ConsumerAction, DispatchResult, Headers, NatsHeaderMap,
    OperationKind, OperationSpec, Request, RequestEndpoint, Response, decode_json, encode_json,
    message, service, testing::TestApp,
};
use serde::{Deserialize, Serialize};

const PAYLOAD: &[u8] = br#"{"name":"Ada"}"#;
const REPLY: &str = "_INBOX.benchmark";
const SUBJECT: &str = "bench.v1.raw";

struct RawEndpoint;

impl RequestEndpoint<()> for RawEndpoint {
    const SPEC: OperationSpec = operation("raw_borrowed_static_response", Codec::Raw, Codec::Raw);

    async fn call<'a>((): &'a (), request: Request<'a>) -> DispatchResult {
        black_box(request.body());
        Ok(Response::bytes(Bytes::from_static(b"ok")))
    }
}

#[derive(Deserialize)]
struct BorrowedInput<'a> {
    name: &'a str,
}

struct BorrowedJsonEndpoint;

impl RequestEndpoint<()> for BorrowedJsonEndpoint {
    const SPEC: OperationSpec = operation("borrowed_json_static_response", Codec::Json, Codec::Raw);

    async fn call<'a>((): &'a (), request: Request<'a>) -> DispatchResult {
        let input: BorrowedInput<'_> = decode_json(&request)?;
        black_box(input.name);
        Ok(Response::bytes(Bytes::from_static(b"ok")))
    }
}

#[derive(Deserialize, Serialize)]
struct OwnedInput {
    name: String,
}

struct OwnedJsonEndpoint;

impl RequestEndpoint<()> for OwnedJsonEndpoint {
    const SPEC: OperationSpec = operation("owned_json_json_response", Codec::Json, Codec::Json);

    async fn call<'a>((): &'a (), request: Request<'a>) -> DispatchResult {
        let input: OwnedInput = decode_json(&request)?;
        encode_json(&input).map(Response::Payload)
    }
}

const fn operation(name: &'static str, request: Codec, response: Codec) -> OperationSpec {
    OperationSpec {
        service_name: "bench",
        service_version: "1.0.0",
        rust_name: name,
        kind: OperationKind::Request,
        subject: SUBJECT,
        subject_template: SUBJECT,
        queue_group: None,
        request_codec: request,
        response_codec: response,
        request_encrypted: false,
        response_encrypted: false,
        request_type: None,
        response_type: None,
        error_type: None,
        auth: AuthPolicy::None,
        concurrency: 1,
        params: &[],
    }
}

#[derive(Clone, PartialEq, nats_micro::prost::Message)]
struct ProtoValue {
    #[prost(string, tag = "1")]
    name: String,
}

#[message]
struct RoundTrip<'a> {
    value: &'a str,
}

#[message]
struct RoundTripOutput {
    value: String,
}

#[service(name = "bench-client", version = "1.0.0")]
impl BenchService {
    #[request("round-trip")]
    async fn round_trip(input: RoundTrip<'_>) -> RoundTripOutput {
        RoundTripOutput {
            value: input.value.to_owned(),
        }
    }
}

fn request_dispatch(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("benchmark runtime");

    criterion.bench_function("raw_borrowed_static_response", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(RawEndpoint::call(
                    &(),
                    Request::new(SUBJECT, Some(REPLY), b"request", None),
                ))
                .expect("static response")
        });
    });

    criterion.bench_function("borrowed_json_static_response", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(BorrowedJsonEndpoint::call(
                    &(),
                    Request::new(SUBJECT, Some(REPLY), PAYLOAD, None),
                ))
                .expect("borrowed JSON response")
        });
    });

    criterion.bench_function("owned_json_json_response", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(OwnedJsonEndpoint::call(
                    &(),
                    Request::new(SUBJECT, Some(REPLY), PAYLOAD, None),
                ))
                .expect("owned JSON response")
        });
    });

    criterion.bench_function("protobuf_round_trip", |bencher| {
        let value = ProtoValue {
            name: "Ada".to_owned(),
        };
        bencher.iter(|| {
            let encoded = nats_micro::encode_proto(black_box(&value)).expect("encode protobuf");
            black_box(
                <ProtoValue as nats_micro::prost::Message>::decode(encoded)
                    .expect("decode protobuf benchmark"),
            )
        });
    });
}

fn extraction_and_subjects(criterion: &mut Criterion) {
    criterion.bench_function("subject_one_param", |bencher| {
        bencher.iter(|| {
            let value = black_box(42_u64);
            let mut subject = String::with_capacity(
                "bench.v1.users.".len() + nats_micro::subject_param_len(&value),
            );
            subject.push_str("bench.v1.users.");
            nats_micro::push_subject_param(&mut subject, &value);
            black_box(subject)
        });
    });

    criterion.bench_function("subject_four_params", |bencher| {
        bencher.iter(|| {
            let values = black_box((1_u64, 2_u64, 3_u64, 4_u64));
            let mut subject = String::with_capacity(32);
            subject.push_str("bench.v1");
            for value in [values.0, values.1, values.2, values.3] {
                subject.push('.');
                nats_micro::push_subject_param(&mut subject, &value);
            }
            black_box(subject)
        });
    });

    let mut raw_headers = NatsHeaderMap::new();
    raw_headers.insert("x-optional", "present");
    let present = Headers::new(Some(&raw_headers));
    let absent = Headers::new(None);

    criterion.bench_function("header_optional_present", |bencher| {
        bencher.iter(|| black_box(present.get(black_box("x-optional"))));
    });
    criterion.bench_function("header_optional_absent", |bencher| {
        bencher.iter(|| black_box(absent.get(black_box("x-optional"))));
    });
}

fn transports_and_consumers(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("benchmark runtime");
    let app = TestApp::stateless().serve(BenchService).start();
    let client = app.client::<BenchService>();

    criterion.bench_function("in_memory_generated_client_round_trip", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(client.round_trip(&RoundTrip { value: "hello" }))
                .expect("in-memory generated client")
        });
    });

    if let Some(live_client) = std::env::var("NATS_URL")
        .ok()
        .and_then(|url| runtime.block_on(live_benchmark_client(&url)))
    {
        criterion.bench_function("live_generated_client_round_trip", |bencher| {
            bencher.iter(|| {
                runtime
                    .block_on(live_client.round_trip(&RoundTrip { value: "hello" }))
                    .expect("live generated client")
            });
        });
    }

    criterion.bench_function("consumer_ack", |bencher| {
        bencher.iter(|| black_box(ConsumerAction::Ack));
    });

    criterion.bench_function("consumer_progress_ack", |bencher| {
        let interval = Duration::from_secs(10);
        bencher.iter(|| black_box(tokio::time::Instant::now() + interval));
    });
}

async fn live_benchmark_client(
    server: &str,
) -> Option<BenchServiceClient<nats_micro::NatsTransport>> {
    let client = nats_micro::async_nats::connect(server).await.ok()?;
    let mut subscription = client
        .subscribe(BenchService::SPEC.operations[0].subject)
        .await
        .ok()?;
    let responder = client.clone();
    tokio::spawn(async move {
        while let Some(message) = subscription.next().await {
            if let Some(reply) = message.reply {
                let _ = responder.publish(reply, message.payload).await;
            }
        }
    });
    Some(BenchServiceClient::new(nats_micro::NatsTransport::new(
        client,
    )))
}

criterion_group!(
    benches,
    request_dispatch,
    extraction_and_subjects,
    transports_and_consumers
);
criterion_main!(benches);
