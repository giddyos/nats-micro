#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&str, &str)| {
    let (pattern, subject) = input;
    let first = nats_micro::subject_matches(pattern, subject);
    let second = nats_micro::subject_matches(pattern, subject);
    assert_eq!(first, second);
});
