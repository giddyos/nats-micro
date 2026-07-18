#[test]
fn borrowed_request_cannot_escape_into_a_spawned_task() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/static_borrowed_request_spawn.rs");
}
