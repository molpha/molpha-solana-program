//! Encoding pins: instruction argument wire formats, the `Node` account layout, and the feed
//! stored-value encoding.
//!
//! Pure unit tests — no VM. Every assertion here guards a shape the CLI, gateway and SDK encode
//! or decode off-chain, so a failure means a breaking change that should be a deliberate one.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AnchorDeserialize, AnchorSerialize, Discriminator, InstructionData, Space};
use molpha::instructions::{InitializeArgs, SubmitAttestationArgs, UpdateConfigArgs};
use molpha::state::{Feed, FeedValueKind, Node, NodeStatus, OffenceKind};
use solana_keccak_hasher::hashv;

/// Borsh encoding of `InitializeArgs`, plus the 8-byte Anchor discriminator.
///
/// Pinned because the CLI, gateway and SDK all build this instruction off-chain: any field
/// added, removed or retyped here is a breaking wire change and should be a deliberate one.
const INITIALIZE_IX_DATA_LEN: usize = 102;

/// Borsh encoding of `UpdateConfigArgs`, plus the 8-byte Anchor discriminator.
/// Pinned for the same reason as `INITIALIZE_IX_DATA_LEN`: off-chain clients encode this.
const UPDATE_CONFIG_IX_DATA_LEN: usize = 69;

/// Borsh encoding of `SubmitAttestationArgs`. Every field is fixed-width, so this is a
/// constant — and a change to it is a breaking change for every off-chain submitter.
const SUBMIT_ATTESTATION_ARGS_LEN: usize = 161;

fn initialize_args() -> InitializeArgs {
    InitializeArgs {
        withdrawal_cooldown_slots: 3,
        redundancy_buffer: 2,
        minimum_node_deposit: 100_000_000,
        reward_per_signature: 10,
        max_settlement_delay_seconds: 256,
        dispute_window_slots: 1024,
        gateway_bond_min: 500_000_000,
        challenger_bounty_bps: 500,
        liveliness_freeze_threshold: 5_000,
        usdc_mint: Pubkey::new_from_array([7; 32]),
        x402_round_base: 1_000,
        min_signers: 3,
    }
}

#[test]
fn initialize_instruction_data_is_stable_and_round_trips() {
    let args = initialize_args();
    let data = molpha::instruction::Initialize { args: args.clone() }.data();
    assert_eq!(data.len(), INITIALIZE_IX_DATA_LEN);

    let decoded = borsh::from_slice::<InitializeArgs>(&data[8..]).expect("deserialize args");
    assert_eq!(
        decoded.withdrawal_cooldown_slots,
        args.withdrawal_cooldown_slots
    );
    assert_eq!(decoded.redundancy_buffer, args.redundancy_buffer);
    assert_eq!(decoded.minimum_node_deposit, args.minimum_node_deposit);
    assert_eq!(decoded.reward_per_signature, args.reward_per_signature);
    assert_eq!(
        decoded.max_settlement_delay_seconds,
        args.max_settlement_delay_seconds
    );
    assert_eq!(decoded.dispute_window_slots, args.dispute_window_slots);
    assert_eq!(decoded.gateway_bond_min, args.gateway_bond_min);
    assert_eq!(decoded.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(
        decoded.liveliness_freeze_threshold,
        args.liveliness_freeze_threshold
    );
    assert_eq!(decoded.usdc_mint, args.usdc_mint);
    assert_eq!(decoded.x402_round_base, args.x402_round_base);
    assert_eq!(decoded.min_signers, args.min_signers);
}

fn update_config_args() -> UpdateConfigArgs {
    UpdateConfigArgs {
        withdrawal_cooldown_slots: 3,
        minimum_node_deposit: 100_000_000,
        reward_per_signature: 10,
        max_settlement_delay_seconds: 256,
        dispute_window_slots: 1024,
        gateway_bond_min: 500_000_000,
        challenger_bounty_bps: 500,
        liveliness_freeze_threshold: 5_000,
        x402_round_base: 1_000,
        min_signers: 3,
    }
}

#[test]
fn update_config_instruction_data_is_stable_and_round_trips() {
    let args = update_config_args();
    let data = molpha::instruction::UpdateConfig { args: args.clone() }.data();
    assert_eq!(data.len(), UPDATE_CONFIG_IX_DATA_LEN);

    let decoded = borsh::from_slice::<UpdateConfigArgs>(&data[8..]).expect("deserialize args");
    assert_eq!(
        decoded.withdrawal_cooldown_slots,
        args.withdrawal_cooldown_slots
    );
    assert_eq!(decoded.minimum_node_deposit, args.minimum_node_deposit);
    assert_eq!(decoded.reward_per_signature, args.reward_per_signature);
    assert_eq!(
        decoded.max_settlement_delay_seconds,
        args.max_settlement_delay_seconds
    );
    assert_eq!(decoded.dispute_window_slots, args.dispute_window_slots);
    assert_eq!(decoded.gateway_bond_min, args.gateway_bond_min);
    assert_eq!(decoded.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(
        decoded.liveliness_freeze_threshold,
        args.liveliness_freeze_threshold
    );
    assert_eq!(decoded.x402_round_base, args.x402_round_base);
    assert_eq!(decoded.min_signers, args.min_signers);
}

fn submit_attestation_args() -> SubmitAttestationArgs {
    SubmitAttestationArgs {
        source_id: [0x11; 32],
        signatures_required: 3,
        registry_version: 7,
        signers_bitmap: [0x33; 32],
        value: [0x44; 32],
        canonical_timestamp: 1_700_000_000,
        agg_sig_s: [0x66; 32],
        commitment: [0x77; 20],
    }
}

#[test]
fn submit_attestation_args_are_stable_and_round_trip() {
    let args = submit_attestation_args();
    let bytes = borsh::to_vec(&args).expect("serialize");
    assert_eq!(bytes.len(), SUBMIT_ATTESTATION_ARGS_LEN);

    let decoded = borsh::from_slice::<SubmitAttestationArgs>(&bytes).expect("deserialize");
    assert_eq!(decoded.source_id, args.source_id);
    assert_eq!(decoded.registry_version, args.registry_version);
    assert_eq!(decoded.signatures_required, args.signatures_required);
    assert_eq!(decoded.signers_bitmap, args.signers_bitmap);
    assert_eq!(decoded.value, args.value);
    assert_eq!(decoded.canonical_timestamp, args.canonical_timestamp);
    assert_eq!(decoded.agg_sig_s, args.agg_sig_s);
    assert_eq!(decoded.commitment, args.commitment);
}

#[test]
fn permanent_node_roundtrips() {
    let account = Node {
        owner: Pubkey::new_from_array([0xaa; 32]),
        secp256k1_pubkey_x: [0x11; 32],
        secp256k1_pubkey_y: [0x22; 32],
        status: NodeStatus::Active,
        ip: [127, 0, 0, 1],
        port: 8080,
        locked_amount: 100,
        claimable_rewards: 9,
        liveliness_score: 9_500,
        registered_at: 1,
        withdrawable_slot: 0,
        frozen_until: 0,
        bump: 7,
    };
    let mut body = Vec::new();
    account.serialize(&mut body).unwrap();
    assert_eq!(body.len(), Node::INIT_SPACE);

    let mut data = [0u8; Node::SPACE];
    data[..8].copy_from_slice(Node::DISCRIMINATOR);
    data[8..].copy_from_slice(&body);
    let decoded = Node::deserialize(&mut &data[8..]).unwrap();
    assert_eq!(decoded.owner, account.owner);
    assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
    assert_eq!(decoded.claimable_rewards, account.claimable_rewards);
    assert_eq!(decoded.liveliness_score, account.liveliness_score);
}

/// The only caller is `submit_attestation`, whose `value` is a fixed `[u8; 32]`.
#[test]
fn a_full_width_value_is_stored_verbatim() {
    let raw = [0x11u8; Feed::STORED_VALUE_LEN];
    let (stored, kind) = Feed::encode_stored_value(&raw).unwrap();
    assert_eq!(kind, FeedValueKind::Value);
    assert_eq!(stored, raw);
}

/// Reachable only if a future instruction submits a variable-length payload. The digest is
/// the same keccak the EVM side computes, so the branch is pinned even while unused.
#[test]
fn a_payload_longer_than_the_slot_is_stored_as_its_keccak_digest() {
    let raw = vec![0x22u8; 64];
    let (stored, kind) = Feed::encode_stored_value(&raw).unwrap();
    assert_eq!(kind, FeedValueKind::Hash);
    assert_eq!(stored.to_vec(), hashv(&[&raw]).to_bytes().to_vec());
}

#[test]
fn a_payload_over_the_maximum_is_rejected() {
    let raw = vec![0u8; Feed::MAX_VALUE_LEN + 1];
    assert!(Feed::encode_stored_value(&raw).is_err());
    assert!(Feed::encode_stored_value(&vec![0u8; Feed::MAX_VALUE_LEN]).is_ok());
}

#[test]
fn offence_kind_discriminants_are_stable() {
    assert_eq!(borsh::to_vec(&OffenceKind::Equivocation).unwrap(), vec![0]);
    assert_eq!(
        borsh::to_vec(&OffenceKind::UnsolicitedSigning).unwrap(),
        vec![1]
    );
    assert_eq!(borsh::to_vec(&OffenceKind::NonceReuse).unwrap(), vec![2]);
}
