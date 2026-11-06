#![no_main]

#[allow(dead_code)]
#[path = "../../molpha-tests/src/security_oracles.rs"]
mod oracles;

libfuzzer_sys::fuzz_target!(|input: &[u8]| oracles::registry_bytes(input));
