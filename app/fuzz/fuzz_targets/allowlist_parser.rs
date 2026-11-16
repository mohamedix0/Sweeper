#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    sweeper_knowledge::fuzz_allowlist_bytes(data);
});
