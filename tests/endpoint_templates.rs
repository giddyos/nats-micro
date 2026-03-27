#[test]
fn endpoint_template_validation() {
    let tests = trybuild::TestCases::new();

    tests.compile_fail("tests/ui/endpoint_missing_params.rs");
    tests.compile_fail("tests/ui/endpoint_extra_param.rs");
    tests.compile_fail("tests/ui/endpoint_loose_function.rs");
    tests.compile_fail("tests/ui/consumer_loose_function.rs");
    tests.pass("tests/ui/endpoint_underscore_params.rs");
    tests.pass("tests/ui/endpoint_custom_param.rs");
    tests.pass("tests/ui/service_handlers_basic.rs");
    tests.pass("tests/ui/service_multi_service.rs");
}
