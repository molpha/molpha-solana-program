//! Encoding pins: instruction argument wire formats, the `Node` and `Feed` account layouts, and
//! the attestation mirror of the verifier types.
//!
//! Pure unit tests — no VM. Every assertion here guards a shape the CLI, gateway and SDK encode
//! or decode off-chain, so a failure means a breaking change that should be a deliberate one.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AnchorDeserialize, AnchorSerialize, Discriminator, InstructionData, Space};
use molpha::instructions::{InitializeArgs, SubmitAttestationArgs, UpdateConfigArgs};
use molpha::state::{
    EpochState, Feed, FeedValueKind, InactivityEvidence, Node, NodeStatus, Offence, OffenceKind,
    TicketMarker,
};
use molpha_verifier::payload::{Attestation, AttestationPayload, SchnorrSignature};
use molpha_verifier::CoalitionKey;

/// Borsh encoding of `InitializeArgs`, plus the 8-byte Anchor discriminator.
///
/// Pinned because the CLI, gateway and SDK all build this instruction off-chain: any field
/// added, removed or retyped here is a breaking wire change and should be a deliberate one.
const INITIALIZE_IX_DATA_LEN: usize = 118;

/// Borsh encoding of `UpdateConfigArgs`, plus the 8-byte Anchor discriminator.
/// Pinned for the same reason as `INITIALIZE_IX_DATA_LEN`: off-chain clients encode this.
const UPDATE_CONFIG_IX_DATA_LEN: usize = 77;

/// Borsh encoding of `SubmitAttestationArgs` without a raw value: the 161-byte attestation, the
/// `None` tag and the 64-byte coalition key. A change to it is a breaking change for every
/// off-chain submitter.
const SUBMIT_ATTESTATION_ARGS_LEN: usize = 226;

fn initialize_args() -> InitializeArgs {
    InitializeArgs {
        withdrawal_cooldown_slots: 3,
        redundancy_buffer: 2,
        minimum_node_deposit: 100_000_000,
        reward_per_signature: 10,
        max_settlement_delay_seconds: 256,
        challenger_bounty_bps: 500,
        usdc_mint: Pubkey::new_from_array([7; 32]),
        x402_round_base: 1_000,
        min_signers: 3,
        epoch_len_seconds: 86_400,
        ticket_grace_seconds: 21_600,
        target_tickets_per_epoch: 2_000,
        min_availability_bps: 8_000,
        min_availability_samples: 50,
        protocol_fee_bps: 500,
        initial_tau: 1 << 40,
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
    assert_eq!(decoded.epoch_len_seconds, args.epoch_len_seconds);
    assert_eq!(decoded.ticket_grace_seconds, args.ticket_grace_seconds);
    assert_eq!(
        decoded.target_tickets_per_epoch,
        args.target_tickets_per_epoch
    );
    assert_eq!(decoded.min_availability_bps, args.min_availability_bps);
    assert_eq!(
        decoded.min_availability_samples,
        args.min_availability_samples
    );
    assert_eq!(decoded.protocol_fee_bps, args.protocol_fee_bps);
    assert_eq!(decoded.initial_tau, args.initial_tau);
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
        x402_round_base: 1_000,
        min_signers: 3,
        epoch_len_seconds: 86_400,
        ticket_grace_seconds: 21_600,
        target_tickets_per_epoch: 2_000,
        min_availability_bps: 8_000,
        min_availability_samples: 50,
        protocol_fee_bps: 500,
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
    assert_eq!(decoded.epoch_len_seconds, args.epoch_len_seconds);
    assert_eq!(decoded.ticket_grace_seconds, args.ticket_grace_seconds);
    assert_eq!(
        decoded.target_tickets_per_epoch,
        args.target_tickets_per_epoch
    );
    assert_eq!(decoded.min_availability_bps, args.min_availability_bps);
    assert_eq!(
        decoded.min_availability_samples,
        args.min_availability_samples
    );
    assert_eq!(decoded.protocol_fee_bps, args.protocol_fee_bps);
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
            timestamp: 1_700_000_000,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0x66; 32],
            commitment: [0x77; 20],
            signers_bitmap: [0x33; 32],
        },
    }
}

fn coalition_key() -> CoalitionKey {
    CoalitionKey {
        x: [0x2a; 32],
        y: [0x3b; 32],
    }
}

/// Borsh `Attestation`: payload (32 + 32 + 4 + 1 + 8) plus signature (32 + 20 + 32).
const ATTESTATION_LEN: usize = 161;

/// `submit_ticket` data: epoch, attestation, roster hints, then the coalition key last, the
/// same trailing-key convention as `submit_attestation`.
#[test]
fn submit_ticket_instruction_ends_with_the_coalition_key() {
    let roster_slots = vec![0u16, 1, 2, 3, 4];
    let data = molpha::instruction::SubmitTicket {
        epoch: 9,
        attestation: attestation(),
        roster_slots: roster_slots.clone(),
        coalition_key: coalition_key(),
    }
    .data();
    assert_eq!(
        data.len(),
        8 + 8 + ATTESTATION_LEN + 4 + 2 * roster_slots.len() + 64
    );
    assert_eq!(data[8..16], 9u64.to_le_bytes());
    assert_eq!(data[data.len() - 64..][..32], [0x2a; 32]);
    assert_eq!(data[data.len() - 32..], [0x3b; 32]);
}

#[test]
fn submit_attestation_args_are_stable_and_round_trip() {
    let args = SubmitAttestationArgs {
        attestation: attestation(),
        raw_value: None,
        coalition_key: coalition_key(),
    };
    let bytes = borsh::to_vec(&args).expect("serialize");
    assert_eq!(bytes.len(), SUBMIT_ATTESTATION_ARGS_LEN);

    let decoded = borsh::from_slice::<SubmitAttestationArgs>(&bytes).expect("deserialize");
    assert_eq!(decoded.attestation, args.attestation);
    assert_eq!(decoded.raw_value, None);
    assert_eq!(decoded.coalition_key, coalition_key());
    // The key trails the variable-length raw value: `x` then `y`, both big-endian.
    assert_eq!(bytes[SUBMIT_ATTESTATION_ARGS_LEN - 64..][..32], [0x2a; 32]);
    assert_eq!(bytes[SUBMIT_ATTESTATION_ARGS_LEN - 32..], [0x3b; 32]);

    let raw = vec![0x5a; 40];
    let with_raw = SubmitAttestationArgs {
        attestation: attestation(),
        raw_value: Some(raw.clone()),
        coalition_key: coalition_key(),
    };
    let bytes = borsh::to_vec(&with_raw).expect("serialize");
    assert_eq!(bytes.len(), SUBMIT_ATTESTATION_ARGS_LEN + 4 + raw.len());
    let decoded = borsh::from_slice::<SubmitAttestationArgs>(&bytes).expect("deserialize");
    assert_eq!(decoded.raw_value, Some(raw));
}

/// The instruction argument of `rotate_node_key`: the new key, the current key's signature `(s,
/// commitment)` and the new key's proof of possession `(r, s)`.
#[test]
fn rotate_node_key_args_wire_format_is_pinned() {
    use molpha_verifier::KeyRotationProof;

    let proof = KeyRotationProof {
        new_public_key_compressed: [0x01; 33],
        old_key_sig_s: [0x02; 32],
        old_key_commitment: [0x03; 20],
        pop_sig_r: [0x04; 32],
        pop_sig_s: [0x05; 32],
    };
    let bytes = borsh::to_vec(&proof).expect("serialize");
    assert_eq!(bytes.len(), 33 + 32 + 20 + 32 + 32);
    assert_eq!(bytes[..33], [0x01; 33]);
    assert_eq!(bytes[33..65], [0x02; 32]);
    assert_eq!(bytes[65..85], [0x03; 20]);
    assert_eq!(bytes[85..117], [0x04; 32]);
    assert_eq!(bytes[117..], [0x05; 32]);

    let data = molpha::instruction::RotateNodeKey { proof }.data();
    assert_eq!(data.len(), 8 + bytes.len());
    assert_eq!(data[8..], bytes[..]);
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
        registered_at: 1,
        withdrawable_slot: 0,
        frozen_until: 0,
        bump: 7,
        key_rotated_at: 0,
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
    assert_eq!(decoded.frozen_until, account.frozen_until);
}

/// `Node` is no longer read by the verifier; off-chain clients still read it by offset, and the
/// rotation timestamp is its last field so every earlier offset is unchanged.
#[test]
fn node_layout_is_pinned() {
    assert_eq!(Node::SPACE, 160);
    let node = Node {
        authority: Pubkey::new_from_array([0xaa; 32]),
        secp256k1_pubkey_x: [0x11; 32],
        secp256k1_pubkey_y: [0x22; 32],
        status: NodeStatus::Frozen,
        ip: [1, 2, 3, 4],
        port: 0x0506,
        locked_amount: 0,
        claimable_rewards: 0,
        registered_at: 0,
        withdrawable_slot: 0,
        frozen_until: 0,
        bump: 0x77,
        key_rotated_at: 0x0102_0304_0506_0708,
    };
    let mut body = Vec::new();
    node.serialize(&mut body).unwrap();
    assert_eq!(body[..32], [0xaa; 32]);
    assert_eq!(body[32..64], [0x11; 32]);
    assert_eq!(body[64..96], [0x22; 32]);
    assert_eq!(body[96], NodeStatus::Frozen as u8);
    assert_eq!(body[97..101], [1, 2, 3, 4]);
    assert_eq!(body[101..103], 0x0506u16.to_le_bytes());
    assert_eq!(body[body.len() - 9], 0x77);
    assert_eq!(
        body[body.len() - 8..],
        0x0102_0304_0506_0708i64.to_le_bytes()
    );
}

/// The shared verifier reads signer keys out of the `Registry` account by offset: a 32-byte header
/// (discriminator, `grace_active_until`, `active_from`, `version`, `node_count`,
/// `redundancy_buffer`, `bump`), then one 96-byte `(node, x, y)` record per member. The account is
/// exactly as long as its members need, and both sides compute that length the same way.
#[test]
fn registry_layout_matches_the_verifier() {
    use anchor_lang::__private::bytemuck;
    use molpha::state::{Registry, RegistryMember, MAX_REGISTRY_NODES};
    use molpha_verifier::solana::{
        registry_account_len, REGISTRY_HEADER_LEN, REGISTRY_MEMBER_LEN, REGISTRY_NODES_OFFSET,
    };

    assert_eq!(Registry::HEADER_SPACE, REGISTRY_HEADER_LEN);
    assert_eq!(REGISTRY_NODES_OFFSET, REGISTRY_HEADER_LEN);
    assert_eq!(RegistryMember::LEN, REGISTRY_MEMBER_LEN);
    assert_eq!(REGISTRY_MEMBER_LEN, molpha_verifier::NodeEntry::LEN);
    assert_eq!(MAX_REGISTRY_NODES, molpha_verifier::MAX_REGISTRY_NODES);
    for members in [0, 1, 12, 106, 107, MAX_REGISTRY_NODES] {
        assert_eq!(Registry::space(members), registry_account_len(members));
        assert_eq!(
            Registry::capacity_of(Registry::space(members)),
            Some(members)
        );
    }
    assert_eq!(Registry::space(0), 32);
    assert_eq!(Registry::space(1), 128);
    // One instruction can create up to 10 KiB: a snapshot of 106 members, and no more.
    assert_eq!(Registry::space(106), 10_208);
    assert_eq!(Registry::space(MAX_REGISTRY_NODES), 24_608);
    // A length that is not a header plus whole records is not a registry.
    for len in [0, 31, 33, 127, 129] {
        assert_eq!(Registry::capacity_of(len), None, "length {len}");
    }

    let header = Registry {
        grace_active_until: 0x1111,
        active_from: 0x2222,
        version: 0x0a0b_0c0d,
        node_count: 0x0102,
        redundancy_buffer: 3,
        bump: 4,
    };
    // After the 8-byte discriminator, which Anchor writes.
    let body = bytemuck::bytes_of(&header);
    assert_eq!(body.len(), REGISTRY_HEADER_LEN - 8);
    assert_eq!(body[..8], 0x1111i64.to_le_bytes());
    assert_eq!(body[8..16], 0x2222i64.to_le_bytes());
    assert_eq!(body[16..20], 0x0a0b_0c0du32.to_le_bytes());
    assert_eq!(body[20..22], 0x0102u16.to_le_bytes());
    assert_eq!(body[22], 3);
    assert_eq!(body[23], 4);

    // A member record is the verifier's `NodeEntry`, byte for byte.
    let members = [
        RegistryMember {
            node: [0x51; 32],
            x: [0x52; 32],
            y: [0x53; 32],
        },
        RegistryMember {
            node: [0x61; 32],
            x: [0x62; 32],
            y: [0x63; 32],
        },
    ];
    let table: &[u8] = bytemuck::cast_slice(&members);
    assert_eq!(table[..32], [0x51; 32]);
    assert_eq!(table[32..64], [0x52; 32]);
    assert_eq!(table[64..96], [0x53; 32]);
    let entries = molpha_verifier::NodeEntry::slice_from_bytes(table).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].account, [0x61; 32]);
    assert_eq!(entries[1].xy(), ([0x62; 32], [0x63; 32]));
}

/// Off-chain clients read `EpochState` as raw zero-copy bytes and derive roster hints from it.
#[test]
fn epoch_and_ticket_layouts_are_pinned() {
    assert_eq!(EpochState::SPACE, 9_384);
    assert_eq!(molpha::state::Registry::space(0), 32);
    assert_eq!(TicketMarker::SPACE, 8 + 8 + 32 + 1);
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

/// `slash_offence` inactivity is the smallest `Offence` variant: one tag byte, the epoch, the
/// roster slot and the current registry location.
/// Pair and nonce variants are larger; pinning this one guards the enum tag and the CLI's
/// inactivity encoder against an accidental reshuffle.
#[test]
fn slash_offence_inactivity_instruction_data_is_stable_and_round_trips() {
    let offence = Offence::Inactivity(InactivityEvidence {
        epoch: 11,
        roster_slot: 5,
        registry_version: 7,
        node_index: 3,
    });
    let data = molpha::instruction::SlashOffence {
        offence: offence.clone(),
    }
    .data();
    assert_eq!(data.len(), 8 + 1 + 8 + 2 + 4 + 4);

    let decoded = borsh::from_slice::<Offence>(&data[8..]).expect("deserialize offence");
    match decoded {
        Offence::Inactivity(evidence) => {
            assert_eq!(evidence.epoch, 11);
            assert_eq!(evidence.roster_slot, 5);
            assert_eq!(evidence.registry_version, 7);
            assert_eq!(evidence.node_index, 3);
        }
        other => panic!("unexpected variant {other:?}"),
    }
    assert_eq!(borsh::to_vec(&offence).unwrap()[0], 4);
}
