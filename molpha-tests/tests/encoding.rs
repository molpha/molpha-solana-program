//! Encoding pins: instruction argument wire formats, the `Node` and `Feed` account layouts, and
//! the attestation mirror of the verifier types.
//!
//! Pure unit tests — no VM. Every assertion here guards a shape the CLI, gateway and SDK encode
//! or decode off-chain, so a failure means a breaking change that should be a deliberate one.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AnchorDeserialize, AnchorSerialize, Discriminator, InstructionData, Space};
use molpha::instructions::{InitializeArgs, SubmitAttestationArgs, UpdateConfigArgs};
use molpha::state::{
    Feed, FeedValueKind, InactivityEvidence, Node, NodeStatus, Offence, OffenceKind,
};
use molpha_verifier::payload::{Attestation, AttestationPayload, SchnorrSignature};

/// Borsh encoding of `InitializeArgs`, plus the 8-byte Anchor discriminator.
///
/// Pinned because the CLI, gateway and SDK all build this instruction off-chain: any field
/// added, removed or retyped here is a breaking wire change and should be a deliberate one.
const INITIALIZE_IX_DATA_LEN: usize = 90;

/// Borsh encoding of `UpdateConfigArgs`, plus the 8-byte Anchor discriminator.
/// Pinned for the same reason as `INITIALIZE_IX_DATA_LEN`: off-chain clients encode this.
const UPDATE_CONFIG_IX_DATA_LEN: usize = 57;

/// Borsh encoding of `SubmitAttestationArgs` without a raw value: the 161-byte attestation plus
/// the `None` tag. A change to it is a breaking change for every off-chain submitter.
const SUBMIT_ATTESTATION_ARGS_LEN: usize = 162;

fn initialize_args() -> InitializeArgs {
    InitializeArgs {
        withdrawal_cooldown_slots: 3,
        redundancy_buffer: 2,
        minimum_node_deposit: 100_000_000,
        reward_per_signature: 10,
        max_settlement_delay_seconds: 256,
        challenger_bounty_bps: 500,
        liveliness_freeze_threshold: 5_000,
        liveliness_penalty_per_miss: 500,
        liveliness_recovery_per_sign: 100,
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
    assert_eq!(decoded.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(
        decoded.liveliness_penalty_per_miss,
        args.liveliness_penalty_per_miss
    );
    assert_eq!(
        decoded.liveliness_recovery_per_sign,
        args.liveliness_recovery_per_sign
    );
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
        challenger_bounty_bps: 500,
        liveliness_freeze_threshold: 5_000,
        liveliness_penalty_per_miss: 500,
        liveliness_recovery_per_sign: 100,
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
    assert_eq!(decoded.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(
        decoded.liveliness_penalty_per_miss,
        args.liveliness_penalty_per_miss
    );
    assert_eq!(
        decoded.liveliness_recovery_per_sign,
        args.liveliness_recovery_per_sign
    );
    assert_eq!(
        decoded.liveliness_freeze_threshold,
        args.liveliness_freeze_threshold
    );
    assert_eq!(decoded.x402_round_base, args.x402_round_base);
    assert_eq!(decoded.min_signers, args.min_signers);
}

fn attestation() -> Attestation {
    Attestation {
        payload: AttestationPayload {
            value: [0x44; 32],
            source_id: [0x11; 32],
            registry_version: 7,
            signatures_required: 3,
            canonical_timestamp: 1_700_000_000,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0x66; 32],
            commitment: [0x77; 20],
            signers_bitmap: [0x33; 32],
        },
    }
}

#[test]
fn submit_attestation_args_are_stable_and_round_trip() {
    let args = SubmitAttestationArgs {
        attestation: attestation(),
        raw_value: None,
    };
    let bytes = borsh::to_vec(&args).expect("serialize");
    assert_eq!(bytes.len(), SUBMIT_ATTESTATION_ARGS_LEN);

    let decoded = borsh::from_slice::<SubmitAttestationArgs>(&bytes).expect("deserialize");
    assert_eq!(decoded.attestation, args.attestation);
    assert_eq!(decoded.raw_value, None);

    let raw = vec![0x5a; 40];
    let with_raw = SubmitAttestationArgs {
        attestation: attestation(),
        raw_value: Some(raw.clone()),
    };
    let bytes = borsh::to_vec(&with_raw).expect("serialize");
    assert_eq!(bytes.len(), SUBMIT_ATTESTATION_ARGS_LEN + 4 + raw.len());
    let decoded = borsh::from_slice::<SubmitAttestationArgs>(&bytes).expect("deserialize");
    assert_eq!(decoded.raw_value, Some(raw));
}

#[test]
fn permanent_node_roundtrips() {
    let account = Node {
        authority: Pubkey::new_from_array([0xaa; 32]),
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
    assert_eq!(decoded.authority, account.authority);
    assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
    assert_eq!(decoded.claimable_rewards, account.claimable_rewards);
    assert_eq!(decoded.liveliness_score, account.liveliness_score);
}

/// Off-chain readers decode `Feed` directly; `value` is a fixed 32-byte slot in either kind.
#[test]
fn feed_layout_is_pinned() {
    assert_eq!(Feed::INIT_SPACE, 32 + 32 + 1 + 8 + 32 + 1 + 32 + 4 + 1);
    assert_eq!(borsh::to_vec(&FeedValueKind::Value).unwrap(), vec![0]);
    assert_eq!(borsh::to_vec(&FeedValueKind::Hash).unwrap(), vec![1]);
}

#[test]
fn offence_discriminants_are_stable() {
    assert_eq!(borsh::to_vec(&OffenceKind::Equivocation).unwrap(), vec![0]);
    assert_eq!(
        borsh::to_vec(&OffenceKind::UnsolicitedSigning).unwrap(),
        vec![1]
    );
    assert_eq!(
        borsh::to_vec(&OffenceKind::CommitmentReuse).unwrap(),
        vec![2]
    );
    assert_eq!(borsh::to_vec(&OffenceKind::NonceReuse).unwrap(), vec![3]);
    assert_eq!(borsh::to_vec(&OffenceKind::Inactivity).unwrap(), vec![4]);
}

/// `slash_offence` inactivity is the smallest `Offence` variant: one tag byte and two `u32`s.
/// Pair and nonce variants are larger; pinning this one guards the enum tag and the CLI's
/// inactivity encoder against an accidental reshuffle.
#[test]
fn slash_offence_inactivity_instruction_data_is_stable_and_round_trips() {
    let offence = Offence::Inactivity(InactivityEvidence {
        registry_version: 7,
        node_index: 3,
    });
    let data = molpha::instruction::SlashOffence {
        offence: offence.clone(),
    }
    .data();
    assert_eq!(data.len(), 8 + 1 + 4 + 4);

    let decoded = borsh::from_slice::<Offence>(&data[8..]).expect("deserialize offence");
    match decoded {
        Offence::Inactivity(evidence) => {
            assert_eq!(evidence.registry_version, 7);
            assert_eq!(evidence.node_index, 3);
        }
        other => panic!("unexpected variant {other:?}"),
    }
    assert_eq!(borsh::to_vec(&offence).unwrap()[0], 4);
}
