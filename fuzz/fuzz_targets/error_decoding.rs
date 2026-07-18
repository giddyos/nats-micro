#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(error) = serde_json::from_slice::<nats_micro::NatsErrorResponse>(input) {
        let encoded = serde_json::to_vec(&error).expect("serialize decoded service error");
        let decoded: nats_micro::NatsErrorResponse =
            serde_json::from_slice(&encoded).expect("decode serialized service error");
        assert_eq!(decoded.code, error.code);
        assert_eq!(decoded.kind, error.kind);
    }
});
