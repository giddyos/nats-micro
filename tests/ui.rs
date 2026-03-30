#[test]
fn macro_ui_validation() {
    let tests = trybuild::TestCases::new();

    tests.compile_fail("tests/ui/endpoint_missing_params.rs");
    tests.compile_fail("tests/ui/endpoint_extra_param.rs");
    tests.compile_fail("tests/ui/consumer_invalid_config.rs");
    tests.compile_fail("tests/ui/consumer_invalid_concurrency_limit.rs");
    tests.compile_fail("tests/ui/endpoint_duplicate_payload.rs");
    tests.pass("tests/ui/endpoint_underscore_params.rs");
    tests.pass("tests/ui/endpoint_custom_param.rs");
    tests.pass("tests/ui/service_multi_auth_extractors.rs");
    tests.pass("tests/ui/service_concurrency_limits.rs");
    tests.pass("tests/ui/service_handlers_basic.rs");
    tests.pass("tests/ui/service_multi_service.rs");
    tests.pass("tests/ui/service_subject_prefix.rs");
    tests.pass("tests/ui/service_client_metadata.rs");
    tests.pass("tests/ui/service_client_generation.rs");
}
