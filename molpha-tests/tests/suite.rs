//! Single test binary for the whole suite.
//!
//! Cargo's default is one executable per file in this directory. Each of those executables
//! statically links LiteSVM and the Agave runtime, and linking several of them at once on a
//! GitHub-hosted runner makes `rust-lld` die with SIGBUS (signal 7). Keeping the cases in
//! separate files but compiling them as modules of this crate links that graph once.
//!
//! Run one area with a filter, for example `cargo test -p molpha-tests --test suite limits`.

mod attestation;
mod compute_bench;
mod delegates;
mod encoding;
mod epoch_rewards;
mod epochs;
mod evm_compatibility;
mod gateway;
mod hashing;
mod limits;
mod node_registry;
mod penalties;
mod plans_and_subscriptions;
mod protocol_config;
mod punish_commitment_reuse;
mod punish_equivocation;
mod punish_limits;
mod punish_nonce_reuse;
mod punish_unsolicited_signing;
mod registry_snapshot;
mod reinstate_node;
mod revenue;
mod slash_offence;
mod tickets;
mod time;
mod withdrawals;

/// A new `tests/*.rs` file that is not listed above is silently dropped from `cargo test`.
#[test]
fn every_test_file_is_a_module() {
    let tests_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let suite_src = std::fs::read_to_string(tests_dir.join("suite.rs")).unwrap();
    for entry in std::fs::read_dir(&tests_dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if name == "suite.rs" || !name.ends_with(".rs") {
            continue;
        }
        let module = name.trim_end_matches(".rs");
        assert!(
            suite_src.contains(&format!("mod {module};")),
            "tests/{name} is not declared in tests/suite.rs"
        );
    }
}
