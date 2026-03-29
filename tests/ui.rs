#[test]
fn macro_ui_validation() {
    let tests = trybuild::TestCases::new();

    tests.compile_fail("tests/ui/endpoint_missing_params.rs");
    tests.compile_fail("tests/ui/endpoint_extra_param.rs");
    tests.compile_fail("tests/ui/consumer_invalid_config.rs");
    tests.pass("tests/ui/endpoint_underscore_params.rs");
    tests.pass("tests/ui/endpoint_custom_param.rs");
    tests.pass("tests/ui/service_handlers_basic.rs");
    tests.pass("tests/ui/service_multi_service.rs");
}
