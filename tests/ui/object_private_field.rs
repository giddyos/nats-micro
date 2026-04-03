#[cfg(feature = "napi")]
#[allow(unused_imports)]
use nats_micro::napi;

#[nats_micro::object]
struct InvalidObject {
    value: String,
}

fn main() {}