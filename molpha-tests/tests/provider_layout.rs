//! The byte layouts documented in `docs/provider-accounts.md`, pinned against the real accounts.
//!
//! The Go decoders (node client, gateway) and the TypeScript SDK follow that file, so a change to
//! any offset here is a wire break and must update the file and those decoders together.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use anchor_lang::{Discriminator, InstructionData};
use molpha::state::{KeyXClaim, ProtocolConfig, Provider, RegistryState};
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

#[test]
fn discriminators_are_the_documented_ones() {
    assert_eq!(hex(Provider::DISCRIMINATOR), "a4b447114bd850c3");
    assert_eq!(hex(KeyXClaim::DISCRIMINATOR), "647254d6c0707103");
    assert_eq!(hex(ProtocolConfig::DISCRIMINATOR), "cf5bfa1c98b3d7d1");
    assert_eq!(hex(RegistryState::DISCRIMINATOR), "1d22e0c3afb76361");

    for (name, data) in [
        // A provider registers, leaves, rotates its key and moves its p2p key through the node
        // instructions.
        (
            "66557572c2bcd3a8",
            molpha::instruction::RegisterNode {
                args: molpha::instructions::RegisterNodeArgs {
                    registration_salt: [0; 32],
                    endpoint: molpha::instructions::MemberEndpoint::Provider { p2p_key: [0; 32] },
                    secp256k1_pubkey_compressed: [0; 33],
                    pop_sig_r: [0; 32],
                    pop_sig_s: [0; 32],
                },
            }
            .data(),
        ),
        (
            "f2b9729216f8179d",
            molpha::instruction::DeactivateNode {
                node_owner: Default::default(),
            }
            .data(),
        ),
        (
            "3f77930b4951e69b",
            molpha::instruction::UpdateNodeEndpoint {
                endpoint: molpha::instructions::MemberEndpoint::Provider { p2p_key: [0; 32] },
            }
            .data(),
        ),
    ] {
        assert_eq!(hex(&data[..8]), name);
    }
}

#[test]
fn the_provider_account_layout() {
    let mut p = Protocol::start();
    p.register_nodes("layout", 3);
    let key = crypto::node_keys("layout-provider", 1).remove(0);
    let index = p.add_active_provider(key.clone());
    let provider = &p.providers[index];
    let data = p.env.account_data(&provider.pda);

    assert_eq!(data.len(), 151);
    assert_eq!(&data[..8], Provider::DISCRIMINATOR);
    assert_eq!(u32_at(&data, 8), provider.id);
    assert_eq!(&data[12..44], provider.authority.pubkey().as_ref());
    assert_eq!(&data[44..76], key.x.as_ref());
    assert_eq!(&data[76..108], key.y.as_ref());
    assert_eq!(&data[108..140], provider.p2p_key.as_ref());
    assert_eq!(data[140], 1, "slot");
    assert_eq!(data[141], 0, "status: 0 = Active");
    let account = p.provider_account(provider.id);
    assert_eq!(data[142], account.bump);
    assert_eq!(i64_at(&data, 143), 0, "key_rotated_at: never rotated");
    assert_eq!(pda::provider(provider.id).to_bytes().len(), 32);
}

#[test]
fn an_ejected_provider_keeps_its_slot_and_is_removed() {
    let mut p = Protocol::start();
    let index = p.register_provider(crypto::node_keys("layout-removed", 1).remove(0));
    p.eject_provider(index);
    let data = p.env.account_data(&p.providers[index].pda);
    assert_eq!((data[140], data[141]), (1, 1), "slot 1, Removed");
}

#[test]
fn the_key_claim_layout() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("layout-claim", 1).remove(0);
    let index = p.register_provider(key.clone());
    let data = p.env.account_data(&pda::key_x_claim(&key.x));
    assert_eq!(data.len(), 41);
    assert_eq!(&data[..8], KeyXClaim::DISCRIMINATOR);
    assert_eq!(&data[8..40], p.providers[index].pda.as_ref());
}

#[test]
fn the_state_layouts_the_decoders_read() {
    let p = Protocol::start();
    // Providers add nothing to the config: `bump` is still the last byte.
    let config = p.env.account_data(&pda::protocol_config());
    assert_eq!(config.len(), 166);
    assert_eq!(config[165], p.config().bump);

    let state = p.env.account_data(&pda::registry_state());
    assert_eq!(state.len(), 21);
    assert_eq!(u32_at(&state, 8), 0, "current_version");
    assert_eq!(u32_at(&state, 12), 1, "next_version");
    assert_eq!(u32_at(&state, 17), 1, "next_provider_id starts at 1");
}

#[test]
fn the_provider_table_follows_the_node_table() {
    let mut p = Protocol::start();
    p.register_nodes("layout-record", 2);
    let first = p.add_active_provider(crypto::node_keys("layout-record-a", 1).remove(0));
    let second = p.add_active_provider(crypto::node_keys("layout-record-b", 1).remove(0));
    let data = p.env.account_data(&pda::registry(p.current_version()));

    // Two node records, then the provider table: the empty reserved slot 0, then slots 1 and 2.
    // Five records, no padding up to bit 64.
    assert_eq!((data[28], data[29]), (2, 3), "node_count, provider_count");
    assert_eq!(data.len(), 32 + 96 * 5);
    for (index, record) in [(first, 3usize), (second, 4usize)] {
        let provider = &p.providers[index];
        // Provider slot `p` is record `node_count + p`, and signs at bit `64 + p`.
        assert_eq!(2 + provider.slot.unwrap(), record);
        assert_eq!(provider.bit(), 64 + provider.slot.unwrap());
        let at = 32 + 96 * record;
        assert_eq!(
            &data[at..at + 32],
            provider.pda.as_ref(),
            "account = Provider PDA"
        );
        assert_eq!(&data[at + 32..at + 64], provider.key.x.as_ref());
        assert_eq!(&data[at + 64..at + 96], provider.key.y.as_ref());
    }
    // Slot 0 (record 2) is vacant: an all-zero record.
    let at = 32 + 96 * 2;
    assert!(data[at..at + 96].iter().all(|byte| *byte == 0));
}
