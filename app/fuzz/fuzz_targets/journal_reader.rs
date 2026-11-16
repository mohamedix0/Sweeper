#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    sweeper_core::fuzz_journal_bytes(data);
});
