use anyhow::anyhow;
use nats_micro::IntoNatsError;
use serde_json::Value;

#[test]
fn anyhow_into_nats_error_includes_truncated_details() {
    let err = anyhow!("something went wrong: secret=very-sensitive-data");
    let resp = err.into_nats_error("req-123".to_string());
    assert_eq!(resp.code, 500);
    assert_eq!(resp.kind, "INTERNAL_ERROR");
    assert_eq!(resp.message, "an internal error occurred");
    assert_eq!(resp.request_id, "req-123");
    assert!(resp.details.is_some());
    if let Some(Value::String(details)) = resp.details {
        assert!(details.contains("something went wrong"));
        assert!(details.len() <= 200);
    }
}
