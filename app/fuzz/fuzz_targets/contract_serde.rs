#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    sweeper_contract::fuzz_contract_payload_bytes(data);
});
