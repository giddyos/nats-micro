use std::future::Future;

use nats_micro::{
    AuthPolicy, Codec, DispatchResult, OperationKind, OperationSpec, Request, RequestEndpoint,
    Response,
};

struct InvalidEndpoint;

impl RequestEndpoint<()> for InvalidEndpoint {
    const SPEC: OperationSpec = OperationSpec {
        rust_name: "invalid",
        kind: OperationKind::Request,
        subject: "invalid.borrow",
        subject_template: "invalid.borrow",
        queue_group: None,
        request_codec: Codec::Raw,
        response_codec: Codec::Empty,
        request_type: Some("&[u8]"),
        response_type: None,
        error_type: None,
        auth: AuthPolicy::None,
        concurrency: 1,
        params: &[],
    };

    fn call<'a>(
        _: &'a (),
        request: Request<'a>,
    ) -> impl Future<Output = DispatchResult> + Send + 'a {
        async move {
            tokio::spawn(async move {
                use_after_request(request.body()).await;
            });
            Ok(Response::empty())
        }
    }
}

async fn use_after_request(_: &[u8]) {}

fn main() {}
