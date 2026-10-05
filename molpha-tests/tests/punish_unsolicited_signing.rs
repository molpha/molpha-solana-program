//! `slash_offence` unsolicited signing: a valid attestation carrying a signer the duty never selected.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{NodePunished, OffenceProven};
use molpha::state::{
    Evidence, Node, NodeStatus, OffenceKind, PunishmentCase, UnsolicitedSigningEvidence,
};
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, events, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 4;

struct Case {
    p: Protocol,
    reporter: Keypair,
    signed: SignedAttestation,
    /// Registry bits that signed without being selected.
    intruders: Vec<usize>,
}

fn duty() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS)
}

/// An attestation from three selected nodes plus one that was never asked.
fn case() -> Case {
    let mut p = Protocol::start();
    p.register_nodes("unsolicited", NODE_COUNT);
    let reporter = p.reporter();

    let duty = duty();
    let selection = p.selection_bits(&duty);
    let intruder = (0..NODE_COUNT)
        .find(|bit| !selection.contains(bit))
        .expect("redundancy_buffer 2 over 8 nodes leaves someone out");

    let mut bits = selection[0..3].to_vec();
    bits.push(intruder);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(duty, &bits, None);

    Case {
        p,
        reporter,
        signed,
        intruders: vec![intruder],
    }
}

fn args_for(_case: &Case, signed: &SignedAttestation) -> UnsolicitedSigningEvidence {
    UnsolicitedSigningEvidence {
        attestation: signed.attestation(),
        coalition_key: signed.coalition_key,
    }
}

fn pda_for_bit(case: &Case, bit: usize) -> Pubkey {
    case.p.env.registry(case.signed.duty.registry_version).nodes[bit]
}

#[test]
fn a_proven_unsolicited_signer_is_slashed_tombstoned_and_evicted_alone() {
    let mut case = case();
    let args = args_for(&case, &case.signed);
    let guilty: Vec<Pubkey> = case
        .intruders
        .iter()
        .map(|b| pda_for_bit(&case, *b))
        .collect();
    let honest_cosigners: Vec<Pubkey> = crypto::bitmap_bits(&case.signed.signers_bitmap)
        .into_iter()
        .filter(|bit| !case.intruders.contains(bit))
        .map(|bit| pda_for_bit(&case, bit))
        .collect();
    assert_eq!(honest_cosigners.len(), 3);

    let locked_each = case.p.args.minimum_node_deposit;
    let expected_slashed = locked_each * guilty.len() as u64;
    let expected_bounty =
        (expected_slashed as u128 * case.p.args.challenger_bounty_bps as u128 / 10_000) as u64;
    let treasury_before = case.p.treasury_balance();

    let reporter = case.reporter.insecure_clone();
    let new_version = case.p.current_version() + 1;
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args.clone());
    let meta = case.p.env.send_ok(&[ix], &[&reporter]);

    for node in &guilty {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
        assert_eq!(account.locked_amount, 0);
    }
    // The nodes that were actually selected co-signed a valid attestation; they did nothing wrong.
    for node in &honest_cosigners {
        let account: Node = case.p.env.get(node);
        assert_eq!(account.status, NodeStatus::Active);
        assert_eq!(account.locked_amount, locked_each);
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
    assert_eq!(record.offence, OffenceKind::UnsolicitedSigning);
    assert_eq!(record.guilty_count, guilty.len() as u16);
    assert_eq!(record.slashed_total, expected_slashed);
    assert_eq!(
        record.guilty_bitmap,
        crypto::bitmap_from_bits(&case.intruders)
    );

    let proven: OffenceProven = only_event(&meta);
    assert_eq!(proven.offence, OffenceKind::UnsolicitedSigning);
    assert_eq!(proven.evidence_hash, args.hash());
    assert_eq!(proven.slashed_total, expected_slashed);
    assert_eq!(events::<NodePunished>(&meta).len(), guilty.len());
}

#[test]
fn a_fully_selected_coalition_is_not_an_offence() {
    let mut p = Protocol::start();
    p.register_nodes("unsolicited-clean", NODE_COUNT);
    let reporter = p.reporter();
    let duty = duty();
    let selection = p.selection_bits(&duty);
    let signed = p.sign_duty_with_bits(duty, &selection[0..4], None);

    let mut case = Case {
        p,
        reporter,
        signed: signed.clone(),
        intruders: vec![],
    };
    let args = args_for(&case, &signed);
    let reporter = case.reporter.insecure_clone();
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersFullySelected);
}

#[test]
fn a_wrong_coalition_key_fails_the_filing_without_punishing_anyone() {
    let mut case = case();
    let mut args = args_for(&case, &case.signed);
    args.coalition_key.y[0] ^= 0x01;
    let reporter = case.reporter.insecure_clone();

    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidCoalitionKey);
    for node in case.p.node_pdas() {
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Active);
    }
}

#[test]
fn an_attestation_whose_signature_does_not_verify_proves_nothing() {
    let mut case = case();
    let tampered = case.signed.with_tampered_signature();
    let args = args_for(&case, &tampered);
    let reporter = case.reporter.insecure_clone();

    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    for node in case.p.node_pdas() {
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Active);
    }
}

#[test]
fn the_same_evidence_cannot_be_filed_twice() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let args = args_for(&case, &case.signed);
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args.clone());
    case.p.env.send_ok(&[ix], &[&reporter]);

    // Same evidence, next registry version: the case PDA is seeded by the evidence hash alone.
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
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
fn filing_requires_the_next_registry_version() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();
    let skipped = case.p.current_version() + 4;
    let ix = case.p.slash_unsolicited_signing_ix_with_registry(
        &reporter.pubkey(),
        args_for(&case, &case.signed),
        skipped,
    );
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

#[test]
fn an_already_tombstoned_intruder_leaves_nothing_to_punish() {
    let mut case = case();
    let reporter = case.reporter.insecure_clone();

    // File once: the intruder is tombstoned.
    let args = args_for(&case, &case.signed);
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter]);

    // A second, distinct offence by the same node (a different timestamp, so different evidence)
    // finds nobody left to slash. Walk timestamps until the tombstoned node is unselected again;
    // it sits outside roughly a quarter of the derived selections, so this settles immediately.
    let intruder = case.intruders[0];
    let (duty, selection) = (0u64..64)
        .map(|offset| {
            let duty = duty().at(duty().timestamp + 1 + offset);
            let selection = case.p.selection_bits(&duty);
            (duty, selection)
        })
        .find(|(_, selection)| !selection.contains(&intruder))
        .expect("some nearby duty leaves the tombstoned node unselected");

    let mut bits: Vec<usize> = selection.iter().copied().take(3).collect();
    bits.push(intruder);
    bits.sort_unstable();

    let signed = case.p.sign_duty_with_bits(duty, &bits, None);
    let args = args_for(&case, &signed);
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    let failed = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::NothingToPunish);
}

#[test]
fn two_intruders_in_one_attestation_are_both_punished() {
    let mut p = Protocol::start();
    // A wider registry so the duty leaves two nodes unselected.
    p.register_nodes("unsolicited-two", 10);
    let reporter = p.reporter();
    let duty = Duty::new(10, SIGS);
    let selection = p.selection_bits(&duty);
    let intruders: Vec<usize> = (0..10).filter(|b| !selection.contains(b)).take(2).collect();
    assert_eq!(intruders.len(), 2);

    let mut bits = selection[0..3].to_vec();
    bits.extend_from_slice(&intruders);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(duty, &bits, None);

    let mut case = Case {
        p,
        reporter,
        signed: signed.clone(),
        intruders: intruders.clone(),
    };
    let args = args_for(&case, &signed);
    let reporter = case.reporter.insecure_clone();
    let ix = case
        .p
        .slash_unsolicited_signing_ix(&reporter.pubkey(), args.clone());
    case.p.env.send_ok(&[ix], &[&reporter]);

    for bit in &intruders {
        let node = pda_for_bit(&case, *bit);
        assert_eq!(case.p.env.get::<Node>(&node).status, NodeStatus::Tombstoned);
    }
    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.hash()));
    assert_eq!(record.guilty_count, 2);
    assert_eq!(record.slashed_total, case.p.args.minimum_node_deposit * 2);
}
