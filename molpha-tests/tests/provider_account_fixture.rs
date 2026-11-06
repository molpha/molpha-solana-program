//! Real account bytes for the Go and TypeScript decoders.
//!
//! The node client and the gateway decode `Provider`, `Registry` and `RegistryState` accounts at
//! fixed offsets (`docs/provider-accounts.md`). This test builds those accounts through the real
//! instructions in LiteSVM and writes their bytes, with the values every field must decode to, to
//! `tests/fixtures/provider_accounts.json`, so a decoder is checked against what the program really
//! produces and not against a second reading of the document.
//!
//! The scenario: three nodes; provider A active at slot 1; provider B active at slot 2 and then
//! ejected (its `Provider` account is `Removed`, and the registry version from before the ejection
//! still holds its record); provider C registered after that, taking the freed slot 2, so a
//! `Removed` provider and an `Active` one both name slot 2 and only the registry says which holds it.
//!
//! Without MOLPHA_WRITE_FIXTURES=1 the test only checks the scenario's own invariants. Run
//! `MOLPHA_WRITE_FIXTURES=1 cargo test -p molpha-tests --test suite provider_account_fixture` to rewrite the
//! file after a layout change, and copy it to the decoders' testdata (the node client's
//! `new/solana/testdata/` and the gateway's `internal/solana/testdata/`).
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use molpha_tests::crypto;
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

fn i64_at(data: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(data[at..at + 8].try_into().unwrap())
}

const STATUS: [&str; 2] = ["Active", "Removed"];

#[test]
fn export_provider_account_bytes() {
    let mut p = Protocol::start();
    p.register_nodes("fixture-nodes", 3);
    let keys = crypto::node_keys("fixture-providers", 3);
    let a = p.add_active_provider(keys[0].clone());
    let b = p.add_active_provider(keys[1].clone());
    let version_with_both = p.current_version();
    p.eject_provider(b);
    let version_after_eject = p.current_version();
    let c = p.register_provider(keys[2].clone());
    let version_with_c = p.current_version();
    assert!(version_after_eject > version_with_both);
    assert_eq!(version_with_c, version_after_eject + 1);

    let registry_with_both = p.env.account_data(&pda::registry(version_with_both));
    let registry_after = p.env.account_data(&pda::registry(version_after_eject));
    let registry_with_c = p.env.account_data(&pda::registry(version_with_c));
    let state = p.env.account_data(&pda::registry_state());

    // The invariants the decoders rely on: three node records, then the provider table (slot 0
    // reserved, then A and B).
    assert_eq!((registry_with_both[28], registry_with_both[29]), (3, 3));
    assert_eq!(
        registry_with_both.len(),
        32 + 96 * 6,
        "three nodes and three provider slots"
    );
    assert_eq!(u32_at(&state, 8), version_with_c, "current_version");
    assert_eq!(
        u32_at(&state, 17),
        4,
        "next_provider_id after three registrations"
    );
    let slot_a = p.providers[a].slot.unwrap();
    let slot_b = p.providers[b].slot.unwrap();
    assert_eq!((slot_a, slot_b), (1, 2));
    let at_b = 32 + 96 * (3 + slot_b);
    assert!(registry_with_both[at_b..at_b + 32] == *p.providers[b].pda.as_ref());
    // B held the last provider slot, so ejecting it trims the provider table to A's slot.
    assert_eq!((registry_after[28], registry_after[29]), (3, 2));
    assert_eq!(
        registry_after.len(),
        32 + 96 * 5,
        "the trailing hole is trimmed"
    );
    // C takes the lowest free provider slot, the one B left.
    assert_eq!(p.providers[c].slot, Some(2));
    assert!(registry_with_c[at_b..at_b + 32] == *p.providers[c].pda.as_ref());

    if std::env::var("MOLPHA_WRITE_FIXTURES").is_err() {
        return;
    }

    let node_json: Vec<String> = p
        .nodes
        .iter()
        .map(|n| {
            let data = p.env.account_data(&n.pda);
            format!(
                "    {{\"address\": \"{}\", \"index\": {}, \"data\": \"{}\", \"x\": \"{}\", \"y\": \"{}\"}}",
                n.pda,
                n.index,
                hex(&data),
                hex(&n.key.x),
                hex(&n.key.y)
            )
        })
        .collect();
    let provider_json: Vec<String> = [(a, "A"), (b, "B"), (c, "C")]
        .into_iter()
        .map(|(index, name)| {
            let pr = &p.providers[index];
            let data = p.env.account_data(&pr.pda);
            format!(
                "    {{\"name\": \"{name}\", \"address\": \"{}\", \"id\": {}, \"status\": \"{}\", \"slot\": {}, \"authority\": \"{}\", \
\"p2p_key\": \"{}\", \"x\": \"{}\", \"y\": \"{}\", \"key_rotated_at\": {}, \"data\": \"{}\"}}",
                pr.pda,
                pr.id,
                STATUS[data[141] as usize],
                data[140],
                pr.authority.pubkey(),
                hex(&pr.p2p_key),
                hex(&pr.key.x),
                hex(&pr.key.y),
                i64_at(&data, 143),
                hex(&data),
            )
        })
        .collect();
    let json = format!(
        "{{\n  \"comment\": \"Real account bytes built by the program's own LiteSVM harness. Regenerate with \
MOLPHA_WRITE_FIXTURES=1 cargo test -p molpha-tests --test suite provider_account_fixture and copy to the node client and the gateway.\",\n  \
\"program_id\": \"{}\",\n  \
\"registry_state\": {{\"address\": \"{}\", \"data\": \"{}\", \"current_version\": {}, \"next_version\": {}, \"next_provider_id\": {}}},\n  \
\"registries\": [\n    {{\"name\": \"with both providers\", \"version\": {}, \"address\": \"{}\", \"data\": \"{}\"}},\n    \
{{\"name\": \"after B is ejected\", \"version\": {}, \"address\": \"{}\", \"data\": \"{}\"}},\n    \
{{\"name\": \"after C registers\", \"version\": {}, \"address\": \"{}\", \"data\": \"{}\"}}\n  ],\n  \
\"nodes\": [\n{}\n  ],\n  \"providers\": [\n{}\n  ]\n}}\n",
        molpha::ID,
        pda::registry_state(),
        hex(&state),
        u32_at(&state, 8),
        u32_at(&state, 12),
        u32_at(&state, 17),
        version_with_both,
        pda::registry(version_with_both),
        hex(&registry_with_both),
        version_after_eject,
        pda::registry(version_after_eject),
        hex(&registry_after),
        version_with_c,
        pda::registry(version_with_c),
        hex(&registry_with_c),
        node_json.join(",\n"),
        provider_json.join(",\n"),
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/provider_accounts.json"
    );
    std::fs::write(path, json).unwrap();
}
