#[test]
fn v2_service_macro_validation() {
    let tests = trybuild::TestCases::new();

    tests.compile_fail("tests/ui/v2_invalid_version.rs");
    tests.compile_fail("tests/ui/v2_invalid_subject.rs");
    tests.compile_fail("tests/ui/v2_duplicate_subjects.rs");
    tests.compile_fail("tests/ui/v2_missing_subject_argument.rs");
    tests.compile_fail("tests/ui/v2_extra_subject_argument.rs");
    tests.compile_fail("tests/ui/v2_duplicate_placeholders.rs");
    tests.compile_fail("tests/ui/v2_multiple_payloads.rs");
    tests.compile_fail("tests/ui/v2_unsupported_payload.rs");
    tests.compile_fail("tests/ui/v2_state_mismatch.rs");
    tests.compile_fail("tests/ui/v2_mutable_state.rs");
    tests.compile_fail("tests/ui/v2_non_send_future.rs");
    tests.compile_fail("tests/ui/v2_borrowed_response.rs");
    tests.compile_fail("tests/ui/v2_wrong_protobuf_traits.rs");
    tests.compile_fail("tests/ui/v2_missing_required_auth.rs");
    tests.compile_fail("tests/ui/v2_optional_auth_mismatch.rs");
    tests.compile_fail("tests/ui/v2_invalid_consumer_backoff.rs");
    tests.compile_fail("tests/ui/v2_zero_concurrency.rs");
    tests.compile_fail("tests/ui/v2_application_const_boundary.rs");
    tests.pass("tests/ui/v3_fluent_startup_boundary.rs");

    #[cfg(feature = "napi")]
    tests.compile_fail("tests/ui/v2_unsupported_napi_type.rs");

    #[cfg(not(feature = "encryption"))]
    tests.compile_fail("tests/ui/v2_encryption_without_feature.rs");
}
