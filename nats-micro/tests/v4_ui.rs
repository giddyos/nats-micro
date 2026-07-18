#![cfg(feature = "test-util")]

#[test]
fn in_memory_test_macro_validation() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/v4_invalid_test_threads.rs");
    tests.pass("tests/ui/v4_test_options.rs");
}
