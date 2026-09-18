//! `slash_offence` equivocation: two valid attestations for one duty with different values.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{NodePunished, OffenceProven};
use molpha::state::{
    EquivocationEvidence, Evidence, MembershipAction, Node, NodeStatus, OffenceKind,
    PunishmentCase,
};
use molpha_verifier::Attestation;
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, events, only_event};
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
    let reporter = p.reporter();

    let duty = duty();
    let selection = p.selection_bits(&duty);
    assert_eq!(selection.len(), SIGS as usize + 2);

    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[2..6], None);
    Case { p, reporter, a, b }
}

fn args_for(_case: &Case, a: &SignedAttestation, b: &SignedAttestation) -> EquivocationEvidence {
    let (attestation_a, attestation_b) = (a.attestation(), b.attestation());
    EquivocationEvidence {
        attestation_a,
        attestation_b,
    }
}

fn equivocation_evidence_hash(a: &Attestation, b: &Attestation) -> [u8; 32] {
    EquivocationEvidence {
        attestation_a: a.clone(),
        attestation_b: b.clone(),
    }
    .hash()
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
    let new_version = case.p.current_version() + 1;
    let ix = case
        .p
        .slash_equivocation_ix(&reporter.pubkey(), args.clone());
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
            account.withdrawable_slot,
            case.p
                .env
                .slot()
                .saturating_add(case.p.args.withdrawal_cooldown_slots)
        );
    }
    // Everyone else is untouched, including the non-overlapping signers.
    for node in &innocent {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Active);
        assert_eq!(account.locked_amount, locked_each);
    }

    // The guilty are evicted into a fresh snapshot; the evidence snapshot is untouched.
    let after = case.p.env.registry(new_version);
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
    assert_eq!(case.p.current_version(), new_version);

    // The bounty is paid out of what the punishment just recovered, in the same instruction.
    let reporter_usdc = pda::ata(&reporter.pubkey(), &case.p.usdc_mint);
    assert_eq!(case.p.env.token_balance(&reporter_usdc), expected_bounty);
    assert_eq!(
        case.p.treasury_balance(),
        treasury_before + expected_slashed - expected_bounty
    );

    // The forfeited principal left the deposit vault; every innocent node's is still there.
    let innocent_total = innocent.len() as u64 * locked_each;
    assert_eq!(case.p.node_deposit_balance(), innocent_total);
    assert_eq!(case.p.total_locked_deposits(), innocent_total);
    for node in &innocent {
        assert_eq!(case.p.env.get::<Node>(node).locked_amount, locked_each);
    }
    case.p.assert_solvent();

    // The case is recorded permanently.

    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.hash()));
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
    assert_eq!(record.evidence_hash, args.hash());
    assert_eq!(record.executed_slot, case.p.env.slot());

    let proven: OffenceProven = only_event(&meta);
    assert_eq!(proven.offence, OffenceKind::Equivocation);
    assert_eq!(proven.evidence_hash, args.hash());
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
        .advance_slots(case.p.args.withdrawal_cooldown_slots);
    let withdraw = case.p.withdraw_node_deposit_ix(&owner.pubkey());
    let failed = case.p.env.send_err(&[withdraw], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToWithdraw);
}

#[test]
fn the_same_evidence_cannot_be_filed_twice_in_either_order() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let args = args_for(&case, &case.a, &case.b);
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    // Swapping the attestations yields the same evidence hash, so the case PDA already exists.
    let swapped = args_for(&case, &case.b, &case.a);
    let first_hash = equivocation_evidence_hash(&case.a.attestation(), &case.b.attestation());
    assert_eq!(swapped.hash(), first_hash);

    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), swapped);
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
fn two_attestations_with_the_same_value_are_not_equivocation() {
    let mut p = Protocol::start();
    p.register_nodes("equivocation-same-value", NODE_COUNT);
    let reporter = p.reporter();
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
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NotEquivocation);
}

#[test]
fn disjoint_coalitions_leave_nobody_to_punish() {
    let mut p = Protocol::start();
    p.register_nodes("equivocation-disjoint", NODE_COUNT);
    let reporter = p.reporter();
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
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::GuiltySetEmpty);
}

#[test]
fn an_attestation_whose_signature_does_not_verify_proves_nothing() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let tampered = case.a.with_tampered_signature();
    let args = args_for(&case, &tampered, &case.b);

    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    for node in case.p.node_pdas() {
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Active);
    }
}

#[test]
fn filing_requires_the_next_registry_version() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let skipped = case.p.current_version() + 2;
    let ix = case.p.slash_equivocation_ix_with_registry(
        &reporter.pubkey(),
        args_for(&case, &case.a, &case.b),
        skipped,
    );
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

#[test]
fn a_punished_node_stops_being_selected_for_later_rounds() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let guilty = guilty_pdas(&case);
    let args = args_for(&case, &case.a, &case.b);
    let new_version = case.p.current_version() + 1;
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
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
    let (a, b) = (case.a.attestation(), case.b.attestation());
    let forward = equivocation_evidence_hash(&a, &b);
    let backward = equivocation_evidence_hash(&b, &a);
    assert_eq!(forward, backward);

    let (mut later_a, mut later_b) = (a.clone(), b.clone());
    later_a.payload.canonical_timestamp += 1;
    later_b.payload.canonical_timestamp += 1;
    assert_ne!(forward, equivocation_evidence_hash(&later_a, &later_b));

    let mut other_signature = a;
    other_signature.signature.agg_sig_s[0] ^= 0x01;
    assert_ne!(forward, equivocation_evidence_hash(&other_signature, &b));

    // Sanity: the two attestations really do differ in their signer sets.
    assert_ne!(case.a.signers_bitmap, case.b.signers_bitmap);
    assert!(!crypto::bitmap_bits(&intersect_bitmap(
        &case.a.signers_bitmap,
        &case.b.signers_bitmap
    ))
    .is_empty());
}

/// Different values for two different duties are just two rounds.
#[test]
fn attestations_for_different_duties_are_not_equivocation() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let later = case.b.duty.at(case.b.duty.canonical_timestamp + 1);
    let bits = crypto::bitmap_bits(&case.b.signers_bitmap);
    let b = case.p.sign_duty_with_bits(later, &bits, None);
    let args = args_for(&case, &case.a, &b);

    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NotEquivocation);
}

/// Both attestations verify against one evidence registry, so they must name the same version.
#[test]
fn attestations_from_different_registry_versions_are_refused() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let authority = case.p.authority.insecure_clone();
    let next = case.p.current_version() + 1;
    let ix = case
        .p
        .update_registry_policy_ix(&authority.pubkey(), next, 4);
    case.p.env.send_ok(&[ix], &[&authority]);

    let b = case.p.sign_duty(
        Duty::new(next, SIGS)
            .source_id(case.a.duty.source_id)
            .at(case.a.duty.canonical_timestamp)
            .value([0x02; 32]),
    );
    let args = args_for(&case, &case.a, &b);

    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NotEquivocation);
}

// ---------------------------------------------------------------------------
// the shared deposit vault
// ---------------------------------------------------------------------------

/// The vault is shared, so the guard against one node's punishment reaching another's principal
/// is arithmetic rather than custody. This pins it: the innocent nodes' deposits are exactly
/// what remains, and the accounting agrees with the balance.
#[test]
fn a_punishment_takes_only_the_guilty_nodes_deposits() {
    let mut case = case();
    let args = args_for(&case, &case.a, &case.b);
    let guilty = guilty_pdas(&case);
    let locked_each = case.p.args.minimum_node_deposit;
    let before = case.p.node_deposit_balance();
    assert_eq!(before, NODE_COUNT as u64 * locked_each);

    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    assert_eq!(
        case.p.node_deposit_balance(),
        before - guilty.len() as u64 * locked_each
    );
    case.p.assert_solvent();

    // And an untouched node can still withdraw its deposit in full out of the shared vault.
    let survivor = case
        .p
        .nodes
        .iter()
        .position(|n| !guilty.contains(&n.pda))
        .expect("someone is innocent");
    let owner = case.p.nodes[survivor].owner.insecure_clone();
    let ix = case.p.deactivate_node_ix(survivor, &owner.pubkey());
    case.p.env.send_ok(&[ix], &[&owner]);
    case.p
        .env
        .advance_slots(case.p.args.withdrawal_cooldown_slots);

    let destination = pda::ata(&owner.pubkey(), &case.p.usdc_mint);
    let paid_before = case.p.env.token_balance_or_zero(&destination);
    let ix = case.p.withdraw_node_deposit_ix(&owner.pubkey());
    case.p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(
        case.p.env.token_balance(&destination),
        paid_before + locked_each
    );
    case.p.assert_solvent();
}

/// A node punished twice is slashed twice, with nothing carried between filings to collide over.
///
/// Unreachable with the shipped penalty table, which tombstones every offence, so the second
/// filing is set up by reviving the node the way a `Freeze` penalty would leave it.
#[test]
fn a_node_can_be_punished_again_after_surviving_an_earlier_punishment() {
    let mut case = case();
    let args = args_for(&case, &case.a, &case.b);
    let guilty = guilty_pdas(&case);
    let locked_each = case.p.args.minimum_node_deposit;

    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    // Half the deposit survives a partial slash. Stand in for one, since no shipped offence
    // leaves a node alive.
    let victim = guilty[0];
    case.p.overwrite_node(&victim, |node| {
        node.status = NodeStatus::Active;
        node.locked_amount = locked_each / 2;
    });
    case.p.fund_usdc(&pda::registry_state(), locked_each / 2);
    case.p.assert_solvent();

    // A second, unrelated filing takes what is left, and no more.
    let second = case.p.sign_duty_with_bits(
        duty().at(1_700_000_900).value([0x07; 32]),
        &case.p.selection_bits(&duty().at(1_700_000_900))[0..4],
        None,
    );
    let third = case.p.sign_duty_with_bits(
        duty().at(1_700_000_900).value([0x08; 32]),
        &case.p.selection_bits(&duty().at(1_700_000_900))[0..4],
        None,
    );
    let vault_before = case.p.node_deposit_balance();
    let args_2 = args_for(&case, &second, &third);
    let guilty_2 = case.p.node_pdas_for_bitmap(
        second.duty.registry_version,
        &intersect_bitmap(&second.signers_bitmap, &third.signers_bitmap),
    );

    if guilty_2.contains(&victim) {
        let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args_2);
        case.p.env.send_ok(&[ix], &[&reporter]);
        assert_eq!(case.p.env.get::<Node>(&victim).locked_amount, 0);
        assert!(case.p.node_deposit_balance() < vault_before);
    }
    case.p.assert_solvent();
}

/// Slashing cancels reward credits, so the protocol stops owing them.
#[test]
fn punishment_releases_the_reward_liability_it_cancels() {
    let mut case = case();
    let gateway = case.p.register_gateway(500_000_000);

    // A settled and finalized round leaves every selected node with a reward credit.
    let round_duty = duty().source_id([0x77; 32]).at(1_700_000_500);
    let signed = case.p.sign_duty(round_duty);
    let x402 = case.p.x402_args(&signed, [0xaa; 32]);
    let record = pda::x402_round(
        &x402.payer_id,
        &pda::gateway(&gateway.pubkey()),
        &round_duty.source_id,
        round_duty.signatures_required,
        round_duty.canonical_timestamp,
    );
    let settle = case
        .p
        .settle_x402_round_ix(&gateway.pubkey(), &signed.attestation(), x402);
    let gateway_kp = gateway.insecure_clone();
    case.p.env.send_ok(&[settle], &[&gateway_kp]);

    let credited = case
        .p
        .node_pdas_for_bitmap(round_duty.registry_version, &signed.signers_bitmap);
    case.p
        .env
        .advance_slots(case.p.args.dispute_window_slots + 1);
    let caller = case.p.env.keypair();
    let ix = case
        .p
        .finalize_round_ix(&caller.pubkey(), &record, &credited);
    case.p.env.send_ok(&[ix], &[&caller]);

    let reward = case.p.args.reward_per_signature;
    let liability_before = case.p.reward_liability();
    assert_eq!(liability_before, reward * credited.len() as u64);

    // Equivocation is a full slash, so each guilty node loses its credit as well as its stake.
    let args = args_for(&case, &case.a, &case.b);
    let guilty = guilty_pdas(&case);
    let credited_guilty = guilty.iter().filter(|n| credited.contains(n)).count() as u64;
    assert!(credited_guilty > 0, "the guilty were paid for this round");

    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_equivocation_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    assert_eq!(
        case.p.reward_liability(),
        liability_before - reward * credited_guilty
    );
    for node in &guilty {
        assert_eq!(case.p.env.get::<Node>(node).claimable_rewards, 0);
    }
    case.p.assert_solvent();
}
