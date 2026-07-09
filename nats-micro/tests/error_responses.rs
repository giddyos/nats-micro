use anyhow::anyhow;
use nats_micro::IntoNatsError;
use serde_json::Value;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn anyhow_into_nats_error_hides_details_by_default() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    unsafe {
        std::env::remove_var("NATS_MICRO_EXPOSE_INTERNAL_ERROR_DETAILS");
    }

    let err = anyhow!("something went wrong: secret=very-sensitive-data");
    let resp = err.into_nats_error("req-123".to_string());

    assert_eq!(resp.code, 500);
    assert_eq!(resp.kind, "INTERNAL_ERROR");
    assert_eq!(resp.message, "an internal error occurred");
    assert_eq!(resp.request_id, "req-123");
    assert!(resp.details.is_none());
}

#[test]
fn anyhow_into_nats_error_can_expose_truncated_details_for_debugging() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    unsafe {
        std::env::set_var("NATS_MICRO_EXPOSE_INTERNAL_ERROR_DETAILS", "true");
    }

    let err = anyhow!("{}", "é".repeat(201));
    let resp = err.into_nats_error("req-utf8".to_string());

    assert_eq!(resp.code, 500);
    assert_eq!(resp.request_id, "req-utf8");
    match resp.details {
        Some(Value::String(details)) => {
            assert_eq!(details.chars().count(), 200);
            assert!(details.is_char_boundary(details.len()));
        }
        other => panic!("expected string details, got {other:?}"),
    }

    unsafe {
        std::env::remove_var("NATS_MICRO_EXPOSE_INTERNAL_ERROR_DETAILS");
    }
}
