#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| git_pr_fuzz::trailers(data));
