//! The fixed penalty table and the arithmetic `apply_penalty` runs on a single node.
//!
//! Pure unit tests — no VM. `punish_equivocation`, `punish_unsolicited_signing` and
//! `punish_commitment_reuse` cover the same penalties end-to-end against the VM.

use anchor_lang::prelude::Pubkey;
use molpha::state::{MembershipAction, Node, NodeStatus, OffenceKind, Penalty};
use molpha::utils::apply_penalty;

fn node_fixture(locked: u64, rewards: u64, status: NodeStatus) -> Node {
    Node {
        owner: Pubkey::new_unique(),
        secp256k1_pubkey_x: [0u8; 32],
        secp256k1_pubkey_y: [0u8; 32],
        status,
        ip: [0, 0, 0, 0],
        port: 0,
        locked_amount: locked,
        claimable_rewards: rewards,
        liveliness_score: molpha::state::LIVELINESS_SCORE_MAX,
        registered_at: 0,
        withdrawable_slot: 0,
        frozen_until: 0,
        bump: 0,
    }
}

#[test]
fn penalties_are_pinned() {
    let equivocation = OffenceKind::Equivocation.penalty();
    assert_eq!(equivocation.slash_bps, 10_000);
    assert_eq!(equivocation.membership, MembershipAction::Tombstone);
    assert_eq!(equivocation.freeze_secs, 0);

    let unsolicited = OffenceKind::UnsolicitedSigning.penalty();
    assert_eq!(unsolicited.slash_bps, 10_000);
    assert_eq!(unsolicited.membership, MembershipAction::Tombstone);
    assert_eq!(unsolicited.freeze_secs, 0);

    let commitment_reuse = OffenceKind::CommitmentReuse.penalty();
    assert_eq!(commitment_reuse.slash_bps, 5_000);
    assert_eq!(commitment_reuse.membership, MembershipAction::Tombstone);
    assert_eq!(commitment_reuse.freeze_secs, 0);
}

#[test]
fn full_slash_zeroes_balances_and_tombstones() {
    let mut node = node_fixture(1_000, 42, NodeStatus::Active);
    let penalty = Penalty {
        slash_bps: 10_000,
        membership: MembershipAction::Tombstone,
        freeze_secs: 0,
    };
    let outcome = apply_penalty(&mut node, penalty, 100).unwrap();
    assert_eq!(outcome.slashed_locked, 1_000);
    assert_eq!(outcome.slashed_rewards, 42);
    assert_eq!(outcome.total(), 1_042);
    assert_eq!(node.locked_amount, 0);
    assert_eq!(node.claimable_rewards, 0);
    assert_eq!(node.status, NodeStatus::Tombstoned);
}

#[test]
fn partial_slash_rounds_down_and_freezes() {
    let mut node = node_fixture(999, 0, NodeStatus::Active);
    let penalty = Penalty {
        slash_bps: 2_500, // 25%
        membership: MembershipAction::Freeze,
        freeze_secs: 3_600,
    };
    let outcome = apply_penalty(&mut node, penalty, 500).unwrap();
    assert_eq!(outcome.slashed_locked, 249); // floor(999 * 0.25)
    assert_eq!(node.locked_amount, 750);
    assert_eq!(node.status, NodeStatus::Frozen);
    assert_eq!(node.frozen_until, 4_100);
}

#[test]
fn zero_bps_is_a_pure_status_transition() {
    let mut node = node_fixture(1_000, 1_000, NodeStatus::Active);
    let penalty = Penalty {
        slash_bps: 0,
        membership: MembershipAction::None,
        freeze_secs: 0,
    };
    let outcome = apply_penalty(&mut node, penalty, 1).unwrap();
    assert_eq!(outcome.slashed_locked, 0);
    assert_eq!(outcome.slashed_rewards, 0);
    assert_eq!(node.locked_amount, 1_000);
    assert_eq!(node.status, NodeStatus::Active);
}

#[test]
fn half_slash_tombstones_and_leaves_the_remainder() {
    let mut node = node_fixture(100_000_001, 3, NodeStatus::Active);
    let penalty = Penalty {
        slash_bps: 5_000, // 50% — OffenceKind::NonceReuse
        membership: MembershipAction::Tombstone,
        freeze_secs: 0,
    };
    let outcome = apply_penalty(&mut node, penalty, 100).unwrap();
    assert_eq!(outcome.slashed_locked, 50_000_000); // floor(100_000_001 * 0.5)
    assert_eq!(outcome.slashed_rewards, 1); // floor(3 * 0.5)
    assert_eq!(node.locked_amount, 50_000_001);
    assert_eq!(node.claimable_rewards, 2);
    assert_eq!(node.status, NodeStatus::Tombstoned);
    assert_eq!(node.frozen_until, 0);
}

#[test]
fn tombstoned_node_cannot_be_punished_again() {
    let mut node = node_fixture(0, 0, NodeStatus::Tombstoned);
    let penalty = Penalty {
        slash_bps: 10_000,
        membership: MembershipAction::Tombstone,
        freeze_secs: 0,
    };
    assert!(apply_penalty(&mut node, penalty, 1).is_err());
}

/// A node can be punished twice, and the second penalty applies to what the first one left.
///
/// Each filing settles its own forfeiture, so there is no carried state for the two to collide
/// over and no way for the pair to forfeit more than the node ever deposited.
#[test]
fn a_second_penalty_accumulates_onto_an_uncranked_seizure() {
    let mut node = node_fixture(1_000, 0, NodeStatus::Active);
    let half = Penalty {
        slash_bps: 5_000,
        membership: MembershipAction::Freeze,
        freeze_secs: 60,
    };
    let first = apply_penalty(&mut node, half, 100).unwrap();
    assert_eq!(first.slashed_locked, 500);
    assert_eq!(node.locked_amount, 500);

    // Nothing has been settled yet, and the node offends again.
    node.status = NodeStatus::Active;
    let full = Penalty {
        slash_bps: 10_000,
        membership: MembershipAction::Tombstone,
        freeze_secs: 0,
    };
    let second = apply_penalty(&mut node, full, 200).unwrap();

    assert_eq!(
        second.slashed_locked, 500,
        "the second penalty applies to what the first one left"
    );
    assert_eq!(node.locked_amount, 0);
    assert_eq!(
        first.slashed_locked + second.slashed_locked,
        1_000,
        "the two penalties together forfeit the whole deposit, and no more"
    );
    assert_eq!(node.status, NodeStatus::Tombstoned);
}
