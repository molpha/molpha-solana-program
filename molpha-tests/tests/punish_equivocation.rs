//! `punish_equivocation`: two valid attestations for one duty with different values.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{EquivocationProven, NodePunished, RegistryVersionCreated};
use molpha::instructions::PunishEquivocationArgs;
use molpha::state::{
    equivocation_evidence_hash, MembershipAction, Node, NodeStatus, OffenceKind, PunishmentCase,
};
use molpha_tests::crypto;
use molpha_tests::env::{assert_molpha_error, events, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{intersect_bitmap, Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 4;

struct Case {
    p: Protocol,
    reporter: Keypair,
    a: SignedAttestation,
    b: SignedAttestation,
}

fn duty() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS)
}

/// Two overlapping coalitions signing different values for the same duty.
fn case() -> Case {
    let mut p = Protocol::start();
    p.register_nodes("equivocation", NODE_COUNT);
    let reporter = p.env.keypair();

    let duty = duty();
    let selection = p.selection_bits(&duty);
    assert_eq!(selection.len(), SIGS as usize + 2);

    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[2..6], None);
    Case { p, reporter, a, b }
}

fn args_for(case: &Case, a: &SignedAttestation, b: &SignedAttestation) -> PunishEquivocationArgs {
    let duty = a.duty;
    let evidence_hash = equivocation_evidence_hash(
        &duty.source_id,
        duty.registry_version,
        duty.signatures_required,
        duty.canonical_timestamp,
        &a.statement(),
        &b.statement(),
    );
    PunishEquivocationArgs {
        source_id: duty.source_id,
        registry_version: duty.registry_version,
        signatures_required: duty.signatures_required,
        canonical_timestamp: duty.canonical_timestamp,
        statement_a: a.statement(),
        statement_b: b.statement(),
        evidence_hash,
        new_registry_version: case.p.current_version() + 1,
    }
}

fn guilty_pdas(case: &Case) -> Vec<Pubkey> {
    let guilty = intersect_bitmap(&case.a.signers_bitmap, &case.b.signers_bitmap);
    case.p
        .node_pdas_for_bitmap(case.a.duty.registry_version, &guilty)
}

#[test]
fn a_proven_equivocation_slashes_tombstones_and_evicts_exactly_the_intersection() {
    let mut case = case();
    let args = args_for(&case, &case.a, &case.b);
    let guilty = guilty_pdas(&case);
    assert_eq!(guilty.len(), 2, "the two coalitions overlap on two nodes");

    let evidence_version = case.a.duty.registry_version;
    let innocent: Vec<Pubkey> = case
        .p
        .node_pdas()
        .into_iter()
        .filter(|n| !guilty.contains(n))
        .collect();
    let locked_each = case.p.args.minimum_node_deposit;
    let expected_slashed = locked_each * guilty.len() as u64;
    let expected_bounty =
        (expected_slashed as u128 * case.p.args.challenger_bounty_bps as u128 / 10_000) as u64;
    let treasury_before = case.p.treasury_balance();

    let reporter = case.reporter.insecure_clone();
    let ix = case
        .p
        .punish_equivocation_ix(&reporter.pubkey(), args.clone());
    let meta = case.p.env.send_ok(&[ix], &[&reporter]);

    // Guilty nodes: fully slashed and tombstoned.
    assert_eq!(OffenceKind::Equivocation.penalty().slash_bps, 10_000);
    assert_eq!(
        OffenceKind::Equivocation.penalty().membership,
        MembershipAction::Tombstone
    );
    for node in &guilty {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
        assert_eq!(account.locked_amount, 0);
        assert_eq!(account.claimable_rewards, 0);
        assert_eq!(
            account.withdrawable_at,
            case.p
                .env
                .unix_timestamp()
                .checked_add(case.p.args.withdrawal_cooldown_secs)
                .unwrap()
        );
    }
    // Everyone else is untouched, including the non-overlapping signers.
    for node in &innocent {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Active);
        assert_eq!(account.locked_amount, locked_each);
    }

    // The guilty are evicted into a fresh snapshot; the evidence snapshot is untouched.
    let after = case.p.env.registry(args.new_registry_version);
    assert_eq!(after.node_count as usize, NODE_COUNT - guilty.len());
    for node in &guilty {
        assert!(!after.nodes.contains(node));
    }
    for node in &innocent {
        assert!(after.nodes.contains(node));
    }
    assert_eq!(
        case.p.env.registry(evidence_version).node_count as usize,
        NODE_COUNT
    );
    assert_eq!(case.p.current_version(), args.new_registry_version);

    // The reporter is paid a bounty out of the treasury.
    let reporter_usdc = pda::ata(&reporter.pubkey(), &case.p.usdc_mint);
    assert_eq!(case.p.env.token_balance(&reporter_usdc), expected_bounty);
    assert_eq!(case.p.treasury_balance(), treasury_before - expected_bounty);

    // The case is recorded permanently.
    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.evidence_hash));
    assert_eq!(record.offence, OffenceKind::Equivocation);
    assert_eq!(record.reporter, reporter.pubkey());
    assert_eq!(record.registry_version, evidence_version);
    assert_eq!(
        record.guilty_bitmap,
        intersect_bitmap(&case.a.signers_bitmap, &case.b.signers_bitmap)
    );
    assert_eq!(record.guilty_count, guilty.len() as u16);
    assert_eq!(record.slashed_total, expected_slashed);
    assert_eq!(record.bounty_paid, expected_bounty);
    assert_eq!(record.evidence_hash, args.evidence_hash);
    assert_eq!(record.executed_slot, case.p.env.slot());

    let proven: EquivocationProven = only_event(&meta);
    assert_eq!(proven.guilty_count, guilty.len() as u16);
    assert_eq!(proven.slashed_total, expected_slashed);
    assert_eq!(proven.bounty_paid, expected_bounty);
    assert_eq!(proven.reporter, reporter.pubkey());

    let punished = events::<NodePunished>(&meta);
    assert_eq!(punished.len(), guilty.len());
    for event in punished {
        assert_eq!(event.offence, OffenceKind::Equivocation);
        assert_eq!(event.new_status, NodeStatus::Tombstoned);
        assert_eq!(event.slashed_locked, locked_each);
    }

    let version_event: RegistryVersionCreated = only_event(&meta);
    assert_eq!(version_event.previous_version, evidence_version);
    assert_eq!(version_event.new_version, args.new_registry_version);

    let fully_slashed = guilty[0];
    let owner = case
        .p
        .nodes
        .iter()
        .find(|registered| registered.pda == fully_slashed)
        .expect("guilty node is registered")
        .owner
        .insecure_clone();
    case.p
        .env
        .advance_seconds(case.p.args.withdrawal_cooldown_secs);
    let withdraw = case.p.withdraw_node_deposit_ix(&owner.pubkey());
    let failed = case.p.env.send_err(&[withdraw], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToWithdraw);
}

#[test]
fn the_same_evidence_cannot_be_filed_twice_in_either_order() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let args = args_for(&case, &case.a, &case.b);
    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    // Swapping the statements yields the same evidence hash, so the case PDA already exists.
    let swapped = args_for(&case, &case.b, &case.a);
    let first_hash = equivocation_evidence_hash(
        &case.a.duty.source_id,
        case.a.duty.registry_version,
        SIGS,
        case.a.duty.canonical_timestamp,
        &case.a.statement(),
        &case.b.statement(),
    );
    assert_eq!(swapped.evidence_hash, first_hash);

    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), swapped);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn two_statements_with_the_same_value_are_not_equivocation() {
    let mut p = Protocol::start();
    p.register_nodes("equivocation-same-value", NODE_COUNT);
    let reporter = p.env.keypair();
    let duty = duty();
    let selection = p.selection_bits(&duty);

    // Different coalitions, identical value: honest redundancy, not a fork.
    let a = p.sign_duty_with_bits(duty, &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty, &selection[2..6], None);
    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);

    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::EquivocationSameStatement);
}

#[test]
fn disjoint_coalitions_leave_nobody_to_punish() {
    let mut p = Protocol::start();
    p.register_nodes("equivocation-disjoint", NODE_COUNT);
    let reporter = p.env.keypair();
    let duty = duty();
    let selection = p.selection_bits(&duty);
    assert_eq!(selection.len(), 6);

    // Two coalitions of three that share no member. Each is below `signatures_required`, which
    // is exactly why an honest protocol never produces this pair — but the guard must still hold.
    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..3], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[3..6], None);
    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);

    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::GuiltySetEmpty);
}

#[test]
fn an_evidence_hash_that_does_not_match_the_statements_is_refused() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let mut args = args_for(&case, &case.a, &case.b);
    args.evidence_hash[0] ^= 0xff;

    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::EvidenceHashMismatch);
}

#[test]
fn a_statement_whose_signature_does_not_verify_proves_nothing() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let tampered = case.a.with_tampered_signature();
    let args = args_for(&case, &tampered, &case.b);

    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    for node in case.p.node_pdas() {
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Active);
    }
}

#[test]
fn a_coalition_outside_the_derived_selection_is_refused() {
    let mut p = Protocol::start();
    p.register_nodes("equivocation-unselected", NODE_COUNT);
    let reporter = p.env.keypair();
    let duty = duty();
    let selection = p.selection_bits(&duty);
    let unselected = (0..NODE_COUNT)
        .find(|bit| !selection.contains(bit))
        .expect("some node is outside the selection");

    let mut bits_b = selection[1..4].to_vec();
    bits_b.push(unselected);
    bits_b.sort_unstable();

    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &bits_b, None);
    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);

    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
}

#[test]
fn filing_requires_the_next_registry_version() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let mut args = args_for(&case, &case.a, &case.b);
    args.new_registry_version += 1;

    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidRegistryVersion);
}

#[test]
fn a_punished_node_stops_being_selected_for_later_rounds() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let guilty = guilty_pdas(&case);
    let args = args_for(&case, &case.a, &case.b);
    let new_version = args.new_registry_version;
    let ix = case.p.punish_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    // A fresh duty against the successor registry can only draw from the survivors.
    let next_duty = Duty::new(new_version, SIGS);
    let bits = case.p.selection_bits(&next_duty);
    let registry = case.p.env.registry(new_version);
    for bit in bits {
        assert!(!guilty.contains(&registry.nodes[bit]));
    }
}

#[test]
fn the_evidence_hash_is_order_independent_but_field_sensitive() {
    let case = case();
    let duty = case.a.duty;
    let forward = equivocation_evidence_hash(
        &duty.source_id,
        duty.registry_version,
        SIGS,
        duty.canonical_timestamp,
        &case.a.statement(),
        &case.b.statement(),
    );
    let backward = equivocation_evidence_hash(
        &duty.source_id,
        duty.registry_version,
        SIGS,
        duty.canonical_timestamp,
        &case.b.statement(),
        &case.a.statement(),
    );
    assert_eq!(forward, backward);

    let other_timestamp = equivocation_evidence_hash(
        &duty.source_id,
        duty.registry_version,
        SIGS,
        duty.canonical_timestamp + 1,
        &case.a.statement(),
        &case.b.statement(),
    );
    assert_ne!(forward, other_timestamp);

    // Sanity: the two statements really do differ in their signer sets.
    assert_ne!(case.a.signers_bitmap, case.b.signers_bitmap);
    assert!(!crypto::bitmap_bits(&intersect_bitmap(
        &case.a.signers_bitmap,
        &case.b.signers_bitmap
    ))
    .is_empty());
}
