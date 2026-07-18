#![cfg(feature = "test-util")]

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use nats_micro::{message, service, testing::TestApp};

#[message]
struct MiriInput<'a> {
    value: &'a str,
}

#[message]
#[derive(Debug)]
struct MiriOutput {
    value: String,
}

#[service(name = "miri-local", version = "2.0.0")]
impl MiriLocalService {
    #[request("echo.{suffix}")]
    async fn echo(suffix: &str, input: MiriInput<'_>) -> MiriOutput {
        MiriOutput {
            value: format!("{}:{suffix}", input.value),
        }
    }
}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("synchronous local route unexpectedly yielded"),
    }
}

#[test]
fn generated_client_and_local_router_are_miri_safe_without_an_io_runtime() {
    let app = TestApp::stateless().serve(MiriLocalService).start();
    let response = ready(
        app.client::<MiriLocalService>()
            .echo("suffix", &MiriInput { value: "miri" }),
    )
    .expect("local generated-client response");

    assert_eq!(response.value, "miri:suffix");
}
