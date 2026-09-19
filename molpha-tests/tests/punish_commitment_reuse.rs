//! `slash_offence` commitment reuse: one aggregate nonce commitment reused across two signed messages.

use molpha::error::MolphaError;
use molpha::events::{NodePunished, OffenceProven};
use molpha::state::{
    CommitmentReuseEvidence, Evidence, MembershipAction, Node, NodeStatus, OffenceKind,
    PunishmentCase,
};
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, events, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{intersect_bitmap, Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 4;
/// The nonce both attestations are signed under — reusing it is the whole offence.
const SHARED_NONCE: [u8; 32] = [0x5c; 32];

struct Case {
    p: Protocol,
    reporter: Keypair,
    a: SignedAttestation,
    b: SignedAttestation,
}

fn duty_a() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS).source_id([0x11; 32])
}

fn duty_b() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS)
        .source_id([0x22; 32])
        .value([0xbb; 32])
}

/// Bits selected for both duties — a coalition that could legitimately sign either.
fn shared_bits(p: &Protocol, a: &Duty, b: &Duty, count: usize) -> Vec<usize> {
    let selection_b = p.selection_bits(b);
    let shared: Vec<usize> = p
        .selection_bits(a)
        .into_iter()
        .filter(|bit| selection_b.contains(bit))
        .collect();
    assert!(
        shared.len() >= count,
        "the two selections overlap on only {} of {count} needed bits",
        shared.len()
    );
    shared[..count].to_vec()
}

/// One coalition signing two different duties under a single nonce.
fn case() -> Case {
    let mut p = Protocol::start();
    p.register_nodes("nonce-reuse", NODE_COUNT);
    let reporter = p.reporter();

    let (a_duty, b_duty) = (duty_a(), duty_b());
    let bits = shared_bits(&p, &a_duty, &b_duty, SIGS as usize);
    let a = p.sign_duty_with_bits(a_duty, &bits, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(b_duty, &bits, Some(SHARED_NONCE));
    assert_eq!(
        a.signature.commitment, b.signature.commitment,
        "a reused nonce must produce a shared commitment"
    );
    assert_ne!(a.message_hash, b.message_hash);

    Case { p, reporter, a, b }
}

fn args_for(_case: &Case, a: &SignedAttestation, b: &SignedAttestation) -> CommitmentReuseEvidence {
    let (attestation_a, attestation_b) = (a.attestation(), b.attestation());
    CommitmentReuseEvidence {
        attestation_a,
        attestation_b,
    }
}

#[test]
fn a_proven_commitment_reuse_halves_the_stake_tombstones_and_evicts_the_intersection() {
    let mut case = case();
    let args = args_for(&case, &case.a, &case.b);
    let guilty_bitmap = intersect_bitmap(&case.a.signers_bitmap, &case.b.signers_bitmap);
    let guilty = case
        .p
        .node_pdas_for_bitmap(case.a.duty.registry_version, &guilty_bitmap);
    assert_eq!(guilty.len(), SIGS as usize);

    let locked_each = case.p.args.minimum_node_deposit;
    // Nonce reuse is a 50% slash, unlike the full slash for equivocation.
    let penalty = OffenceKind::CommitmentReuse.penalty();
    assert_eq!(penalty.slash_bps, 5_000);
    assert_eq!(penalty.membership, MembershipAction::Tombstone);
    let expected_slashed = locked_each / 2 * guilty.len() as u64;
    let expected_bounty =
        (expected_slashed as u128 * case.p.args.challenger_bounty_bps as u128 / 10_000) as u64;
    let treasury_before = case.p.treasury_balance();

    let reporter = case.reporter.insecure_clone();
    let new_version = case.p.current_version() + 1;
    let ix = case
        .p
        .slash_commitment_reuse_ix(&reporter.pubkey(), args.clone());
    let meta = case.p.env.send_ok(&[ix], &[&reporter]);

    for node in &guilty {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
        assert_eq!(
            account.locked_amount,
            locked_each - locked_each / 2,
            "half the deposit survives the slash"
        );
        assert_eq!(
            account.withdrawable_slot,
            case.p
                .env
                .slot()
                .saturating_add(case.p.args.withdrawal_cooldown_slots)
        );
    }
    for node in case.p.node_pdas() {
        if !guilty.contains(&node) {
            let account: Node = case.p.env.get(&node);
            assert_eq!(account.status, NodeStatus::Active);
            assert_eq!(account.locked_amount, locked_each);
        }
    }

    let after = case.p.env.registry(new_version);
    assert_eq!(after.node_count as usize, NODE_COUNT - guilty.len());
    for node in &guilty {
        assert!(!after.nodes.contains(node));
    }

    // The bounty is paid out of what the punishment just recovered, in the same instruction.
    let reporter_usdc = pda::ata(&reporter.pubkey(), &case.p.usdc_mint);
    assert_eq!(case.p.env.token_balance(&reporter_usdc), expected_bounty);
    assert_eq!(
        case.p.treasury_balance(),
        treasury_before + expected_slashed - expected_bounty
    );
    assert_eq!(
        case.p.node_deposit_balance(),
        case.p.total_locked_deposits(),
        "the vault matches what is still owed to nodes"
    );
    case.p.assert_solvent();

    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.hash()));
    assert_eq!(record.offence, OffenceKind::CommitmentReuse);
    assert_eq!(record.guilty_bitmap, guilty_bitmap);
    assert_eq!(record.guilty_count, guilty.len() as u16);
    assert_eq!(record.slashed_total, expected_slashed);
    assert_eq!(record.bounty_paid, expected_bounty);

    let proven: OffenceProven = only_event(&meta);
    assert_eq!(proven.offence, OffenceKind::CommitmentReuse);
    assert_eq!(proven.evidence_hash, args.hash());
    assert_eq!(proven.slashed_total, expected_slashed);
    assert_eq!(events::<NodePunished>(&meta).len(), guilty.len());

    case.p
        .env
        .advance_slots(case.p.args.withdrawal_cooldown_slots);
    for node in &guilty {
        let owner = case
            .p
            .nodes
            .iter()
            .find(|registered| registered.pda == *node)
            .expect("guilty node is registered")
            .owner
            .insecure_clone();
        let survivor = case.p.env.get::<Node>(node).locked_amount;
        let destination = pda::ata(&owner.pubkey(), &case.p.usdc_mint);
        let before = case.p.env.token_balance(&destination);
        let ix = case.p.withdraw_node_deposit_ix(&owner.pubkey());
        case.p.env.send_ok(&[ix], &[&owner]);
        assert_eq!(case.p.env.token_balance(&destination), before + survivor);
        assert_eq!(case.p.env.get::<Node>(node).locked_amount, 0);
    }
}

#[test]
fn attestations_that_do_not_share_a_commitment_are_not_commitment_reuse() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-independent", NODE_COUNT);
    let reporter = p.reporter();
    let (a_duty, b_duty) = (duty_a(), duty_b());
    let bits = shared_bits(&p, &a_duty, &b_duty, SIGS as usize);

    // Independent nonces: exactly what honest signing does.
    let a = p.sign_duty_with_bits(a_duty, &bits, Some([0x01; 32]));
    let b = p.sign_duty_with_bits(b_duty, &bits, Some([0x02; 32]));
    assert_ne!(a.signature.commitment, b.signature.commitment);

    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);
    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NonceCommitmentMismatch);
}

#[test]
fn one_attestation_filed_against_itself_is_not_commitment_reuse() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let args = args_for(&case, &case.a, &case.a);
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NonceReuseSameMessage);
}

#[test]
fn same_duty_with_different_values_must_be_filed_as_equivocation() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-prefer-equivocation", NODE_COUNT);
    let reporter = p.reporter();
    let duty = duty_a();
    let bits = p.selection_bits(&duty)[..SIGS as usize].to_vec();

    // Identical duty, different values, one nonce: that is a fork, not commitment reuse.
    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &bits, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &bits, Some(SHARED_NONCE));

    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);
    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::PreferEquivocation);
}

#[test]
fn the_same_duty_with_the_same_value_but_different_coalitions_is_commitment_reuse() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-same-duty", NODE_COUNT);
    let reporter = p.reporter();
    let duty = duty_a();
    let selection = p.selection_bits(&duty);

    // Same duty tuple and value, different signer sets: the message hashes differ because the
    // bitmap is part of the message, so `PreferEquivocation` does not apply.
    let a = p.sign_duty_with_bits(duty, &selection[0..4], Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(duty, &selection[1..5], Some(SHARED_NONCE));
    assert_eq!(a.signature.commitment, b.signature.commitment);

    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);
    let expected_guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);

    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case
        .p
        .slash_commitment_reuse_ix(&reporter.pubkey(), args.clone());
    case.p.env.send_ok(&[ix], &[&reporter]);

    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.hash()));
    assert_eq!(record.guilty_bitmap, expected_guilty);
    assert_eq!(record.guilty_count, 3);
}

/// Both aggregates verify against one evidence registry, so they must name the same version.
#[test]
fn attestations_from_different_registry_versions_are_refused() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let mut args = args_for(&case, &case.a, &case.b);
    args.attestation_b.payload.registry_version += 1;

    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::RegistryVersionMismatch);
}

/// Reusing a nonce is the offence; neither aggregate has to reach `signatures_required`.
#[test]
fn a_coalition_below_quorum_is_still_punishable() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-below-quorum", NODE_COUNT);
    let reporter = p.reporter();
    let (a_duty, b_duty) = (duty_a(), duty_b());
    let bits = shared_bits(&p, &a_duty, &b_duty, SIGS as usize - 1);

    let a = p.sign_duty_with_bits(a_duty, &bits, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(b_duty, &bits, Some(SHARED_NONCE));
    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);

    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case
        .p
        .slash_commitment_reuse_ix(&reporter.pubkey(), args.clone());
    case.p.env.send_ok(&[ix], &[&reporter]);

    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.hash()));
    assert_eq!(record.guilty_count, SIGS as u16 - 1);
}

#[test]
fn disjoint_coalitions_leave_nobody_to_punish() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-disjoint", NODE_COUNT);
    let reporter = p.reporter();
    let (a_duty, b_duty) = (duty_a(), duty_b());
    let shared = shared_bits(&p, &a_duty, &b_duty, 4);

    // Two coalitions drawn from the shared selection that do not overlap.
    let a = p.sign_duty_with_bits(a_duty, &shared[0..2], Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(b_duty, &shared[2..4], Some(SHARED_NONCE));
    // The commitment is `address(k*G)` — a function of the nonce alone — so the two attestations
    // still collide on it. Only the guilty set is empty.
    assert_eq!(a.signature.commitment, b.signature.commitment);

    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);
    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::GuiltySetEmpty);
}

#[test]
fn an_attestation_whose_signature_does_not_verify_proves_nothing() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let tampered = case.b.with_tampered_signature();
    let args = args_for(&case, &case.a, &tampered);
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    for node in case.p.node_pdas() {
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Active);
    }
}

#[test]
fn a_coalition_outside_its_own_duty_selection_is_refused() {
    let mut p = Protocol::start();
    p.register_nodes("nonce-unselected", NODE_COUNT);
    let reporter = p.reporter();
    let (a_duty, b_duty) = (duty_a(), duty_b());
    let shared = shared_bits(&p, &a_duty, &b_duty, 3);
    let selection_b = p.selection_bits(&b_duty);
    let intruder = (0..NODE_COUNT)
        .find(|bit| !selection_b.contains(bit))
        .expect("some node is outside duty B's selection");

    let mut bits_b = shared.clone();
    bits_b.push(intruder);
    bits_b.sort_unstable();

    let a = p.sign_duty_with_bits(a_duty, &shared, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(b_duty, &bits_b, Some(SHARED_NONCE));

    let case = Case {
        p,
        reporter,
        a: a.clone(),
        b: b.clone(),
    };
    let args = args_for(&case, &a, &b);
    let mut case = case;
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
}

#[test]
fn the_same_evidence_cannot_be_filed_twice_with_the_pairs_swapped() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let args = args_for(&case, &case.a, &case.b);
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    let swapped = args_for(&case, &case.b, &case.a);
    let ix = case
        .p
        .slash_commitment_reuse_ix(&reporter.pubkey(), swapped);
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
fn a_second_filing_does_not_compound_the_slash_on_an_already_tombstoned_node() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let guilty_bitmap = intersect_bitmap(&case.a.signers_bitmap, &case.b.signers_bitmap);
    let guilty = case
        .p
        .node_pdas_for_bitmap(case.a.duty.registry_version, &guilty_bitmap);

    let args = args_for(&case, &case.a, &case.b);
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);
    let balances: Vec<u64> = guilty
        .iter()
        .map(|n| case.p.env.get::<Node>(n).locked_amount)
        .collect();

    // A second, genuinely distinct nonce-reuse case over the same (now tombstoned) coalition.
    // Walk timestamps until both duties select every member of that coalition.
    let bits = crypto::bitmap_bits(&guilty_bitmap);
    let (a_duty, b_duty) = (1u64..256)
        .map(|offset| {
            (
                duty_a().at(duty_a().canonical_timestamp + offset),
                duty_b().at(duty_b().canonical_timestamp + offset),
            )
        })
        .find(|(a_duty, b_duty)| {
            let selection_a = case.p.selection_bits(a_duty);
            let selection_b = case.p.selection_bits(b_duty);
            bits.iter()
                .all(|b| selection_a.contains(b) && selection_b.contains(b))
        })
        .expect("some pair of duties selects the whole tombstoned coalition");

    let a = case.p.sign_duty_with_bits(a_duty, &bits, Some([0x77; 32]));
    let b = case.p.sign_duty_with_bits(b_duty, &bits, Some([0x77; 32]));
    let args = args_for(&case, &a, &b);
    let ix = case.p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NothingToPunish);

    for (node, expected) in guilty.iter().zip(balances) {
        assert_eq!(case.p.env.get::<Node>(node).locked_amount, expected);
    }
}

#[test]
fn filing_requires_the_next_registry_version() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let skipped = case.p.current_version() + 3;
    let ix = case.p.slash_commitment_reuse_ix_with_registry(
        &reporter.pubkey(),
        args_for(&case, &case.a, &case.b),
        skipped,
    );
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}
