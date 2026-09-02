//! How large a guilty set one punishment transaction can carry.
//!
//! Every punishment names its signer `Node` accounts and then one deposit vault per guilty node,
//! so a filing costs 33 transaction bytes per account on both halves. The transaction-size limit
//! binds long before anything else, and reporters need to know where it lands: past it, a filing
//! is only broadcastable as a v0 transaction with an address lookup table.
//!
//! LiteSVM does not enforce the wire limit, so these tests measure rather than expecting a
//! failed send, and they measure with the reporter as fee payer — the way a filing is really sent.
//!
//! The production round shape we must remain able to punish is 18 signatures required plus 2
//! redundancy buffer — 20 signers in the derived selection. Legacy wire limits only allow filing
//! a handful of guilty nodes without a v0 transaction; the execution tests below are the guard
//! that a full 20-node coalition still lands on-chain (unsolicited signing tests the same round
//! shape with the maximum intruder count that fits one transaction).

use molpha::instructions::{
    PunishEquivocationArgs, PunishNonceReuseArgs, PunishUnsolicitedSigningArgs,
};
use molpha::state::{
    equivocation_evidence_hash, nonce_reuse_evidence_hash, unsolicited_signing_evidence_hash,
    Node, NodeStatus,
};
use molpha_tests::crypto;
use molpha_tests::env::PACKET_DATA_SIZE;
use molpha_tests::scenario::{
    intersect_bitmap, unsolicited_guilty_bitmap, Duty, Protocol, SignedAttestation,
};
use solana_signer::Signer;

/// Registry size for legacy wire-size sweeps — wide enough for coalition probes up to 12.
const NODE_COUNT: usize = 24;
const REDUNDANCY_BUFFER: u8 = 2;

/// Round signer set: 18 threshold + 2 redundancy buffer.
const SIGS_REQUIRED: u8 = 18;
const ROUND_SIGNER_COUNT: usize = SIGS_REQUIRED as usize + REDUNDANCY_BUFFER as usize;

/// Intruders punishable in one unsolicited filing alongside a 20-signer round attestation.
///
/// Fixed accounts (11) + accused signers (20 + N) + guilty vaults (N) must stay within runtime
/// account-lock limits in LiteSVM as well as the 64-account wire cap.
const MAX_UNSOLICITED_INTRUDERS: usize = 8;
const UNSOLICITED_REGISTRY_NODES: usize = ROUND_SIGNER_COUNT + MAX_UNSOLICITED_INTRUDERS;

/// Largest fully-overlapping coalition whose `punish_equivocation` filing fits a legacy packet.
///
/// Asserted exactly: adding an account to the instruction, or growing its args, costs a node on
/// both halves of `remaining_accounts` and therefore shrinks this number twice as fast.
const MAX_LEGACY_EQUIVOCATION: usize = 6;

/// Same, for `punish_nonce_reuse`, whose two signing contexts make its args the largest.
const MAX_LEGACY_NONCE_REUSE: usize = 5;

fn protocol() -> Protocol {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("punish-limits", NODE_COUNT);
    p
}

fn round_protocol(node_count: usize) -> Protocol {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("punish-limits-round", node_count);
    p
}

fn assert_round_selection(p: &Protocol, duty: &Duty) {
    let selection = p.selection_bits(duty);
    assert_eq!(
        selection.len(),
        ROUND_SIGNER_COUNT,
        "duty must name the 18+2 round signer set"
    );
}

/// Two values signed by the same `coalition` bits — the worst case, where every signer is guilty.
fn equivocation_pair(
    p: &Protocol,
    coalition: usize,
    at: u64,
) -> (SignedAttestation, SignedAttestation) {
    let duty = Duty::new(
        p.current_version(),
        (coalition - REDUNDANCY_BUFFER as usize) as u8,
    )
    .at(at);
    let bits = p.selection_bits(&duty);
    assert_eq!(bits.len(), coalition);
    (
        p.sign_duty_with_bits(duty.value([0x01; 32]), &bits, None),
        p.sign_duty_with_bits(duty.value([0x02; 32]), &bits, None),
    )
}

fn equivocation_args(a: &SignedAttestation, b: &SignedAttestation) -> PunishEquivocationArgs {
    let duty = a.duty;
    PunishEquivocationArgs {
        source_id: duty.source_id,
        registry_version: duty.registry_version,
        signatures_required: duty.signatures_required,
        canonical_timestamp: duty.canonical_timestamp,
        statement_a: a.statement(),
        statement_b: b.statement(),
        evidence_hash: equivocation_evidence_hash(
            &duty.source_id,
            duty.registry_version,
            duty.signatures_required,
            duty.canonical_timestamp,
            &a.statement(),
            &b.statement(),
        ),
    }
}

/// Fully overlapping nonce reuse over the full round signer set — two duties with the same
/// `source_id` but different timestamps whose selections intersect on `ROUND_SIGNER_COUNT` bits.
fn nonce_reuse_round_pair(p: &Protocol) -> (SignedAttestation, SignedAttestation) {
    const SHARED_NONCE: [u8; 32] = [0x5c; 32];
    let duty_a = Duty::new(p.current_version(), SIGS_REQUIRED).source_id([0x11; 32]);
    let selection_a = p.selection_bits(&duty_a);
    assert_eq!(selection_a.len(), ROUND_SIGNER_COUNT);

    // With exactly `ROUND_SIGNER_COUNT` registry nodes every duty selects the full set, so any
    // second timestamp yields the same 20-node coalition for reuse evidence.
    let duty_b = duty_a.value([0xbb; 32]).at(duty_a.canonical_timestamp + 1);
    let selection_b = p.selection_bits(&duty_b);
    assert_eq!(selection_b.len(), ROUND_SIGNER_COUNT);

    let signing_bits = selection_a;
    let a = p.sign_duty_with_bits(duty_a, &signing_bits, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(duty_b, &signing_bits, Some(SHARED_NONCE));
    (a, b)
}

fn nonce_reuse_args(a: &SignedAttestation, b: &SignedAttestation) -> PunishNonceReuseArgs {
    let duty_a = a.duty;
    PunishNonceReuseArgs {
        registry_version: duty_a.registry_version,
        context_a: a.signing_context(),
        statement_a: a.statement(),
        context_b: b.signing_context(),
        statement_b: b.statement(),
        evidence_hash: nonce_reuse_evidence_hash(
            duty_a.registry_version,
            &a.signing_context(),
            &a.statement(),
            &b.signing_context(),
            &b.statement(),
        ),
    }
}

fn unsolicited_round_statement(p: &Protocol, node_count: usize) -> (SignedAttestation, Vec<usize>) {
    let duty = Duty::new(p.current_version(), SIGS_REQUIRED);
    let selection = p.selection_bits(&duty);
    assert_round_selection(p, &duty);
    let intruders: Vec<usize> = (0..node_count)
        .filter(|bit| !selection.contains(bit))
        .collect();
    let mut bits = selection;
    bits.extend(&intruders);
    bits.sort_unstable();
    let statement = p.sign_duty_with_bits(duty, &bits, None);
    (statement, intruders)
}

fn unsolicited_args(statement: &SignedAttestation) -> PunishUnsolicitedSigningArgs {
    let duty = statement.duty;
    PunishUnsolicitedSigningArgs {
        source_id: duty.source_id,
        registry_version: duty.registry_version,
        signatures_required: duty.signatures_required,
        canonical_timestamp: duty.canonical_timestamp,
        statement: statement.statement(),
        evidence_hash: unsolicited_signing_evidence_hash(
            &duty.source_id,
            duty.registry_version,
            duty.signatures_required,
            duty.canonical_timestamp,
            &statement.statement(),
        ),
    }
}

#[test]
fn a_fully_overlapping_equivocation_fits_a_legacy_packet_up_to_the_pinned_coalition() {
    let mut p = protocol();
    let reporter = p.reporter().pubkey();

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 3..=12 {
        let (a, b) = equivocation_pair(&p, coalition, 1_700_100_000 + coalition as u64);
        let args = equivocation_args(&a, &b);
        let ix = p.punish_equivocation_ix(&reporter, args);
        let size = p.env.transaction_size_as(&reporter, &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("punish_equivocation: {coalition:>2} signers, all guilty -> {size} bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_EQUIVOCATION,
        "the guilty set a legacy equivocation filing can carry changed"
    );
}

#[test]
fn the_largest_fitting_equivocation_filing_actually_lands() {
    let mut p = protocol();
    let reporter = p.reporter();

    let (a, b) = equivocation_pair(&p, MAX_LEGACY_EQUIVOCATION, 1_700_200_500);
    let args = equivocation_args(&a, &b);
    let ix = p.punish_equivocation_ix(&reporter.pubkey(), args);
    assert!(
        p.env
            .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix))
            <= PACKET_DATA_SIZE
    );
    p.env.send_ok(&[ix], &[&reporter]);
}

#[test]
fn a_fully_overlapping_nonce_reuse_fits_a_legacy_packet_up_to_the_pinned_coalition() {
    let mut p = protocol();
    let reporter = p.reporter().pubkey();
    const SHARED_NONCE: [u8; 32] = [0x5c; 32];

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 3..=12 {
        // Wire size only: what matters is the account list and the args, so the coalition is
        // chosen directly rather than from the two duties' overlapping selections, which for a
        // wide registry rarely intersect on enough nodes to reach the ceiling.
        let bits: Vec<usize> = (0..coalition).collect();
        let signatures_required = (coalition - REDUNDANCY_BUFFER as usize) as u8;
        let duty_a = Duty::new(p.current_version(), signatures_required).source_id([0x11; 32]);
        let duty_b = Duty::new(p.current_version(), signatures_required)
            .source_id([0x22; 32])
            .value([0xbb; 32]);
        let a = p.sign_duty_with_bits(duty_a, &bits, Some(SHARED_NONCE));
        let b = p.sign_duty_with_bits(duty_b, &bits, Some(SHARED_NONCE));
        let args = PunishNonceReuseArgs {
            registry_version: duty_a.registry_version,
            context_a: a.signing_context(),
            statement_a: a.statement(),
            context_b: b.signing_context(),
            statement_b: b.statement(),
            evidence_hash: nonce_reuse_evidence_hash(
                duty_a.registry_version,
                &a.signing_context(),
                &a.statement(),
                &b.signing_context(),
                &b.statement(),
            ),
        };
        let ix = p.punish_nonce_reuse_ix(&reporter, args);
        let size = p.env.transaction_size_as(&reporter, &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("punish_nonce_reuse: {coalition:>2} signers, all guilty -> {size} bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_NONCE_REUSE,
        "the guilty set a legacy nonce-reuse filing can carry changed"
    );
}

#[test]
fn a_fully_overlapping_equivocation_with_20_round_signers_executes() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = equivocation_pair(&p, ROUND_SIGNER_COUNT, 1_700_300_000);
    assert_round_selection(&p, &a.duty);
    let args = equivocation_args(&a, &b);
    let guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);
    assert_eq!(crypto::bitmap_bits(&guilty).len(), ROUND_SIGNER_COUNT);
    let guilty_pdas = p.node_pdas_for_bitmap(args.registry_version, &guilty);

    let ix = p.punish_equivocation_ix(&reporter.pubkey(), args);
    let size = p.env.transaction_size_as(&reporter.pubkey(), &[ix.clone()]);
    assert!(
        size > PACKET_DATA_SIZE,
        "20 guilty nodes exceed a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    for node in &guilty_pdas {
        let account: Node = p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
    }
    let after = p.env.registry(p.current_version());
    assert_eq!(
        after.node_count as usize,
        0,
        "every guilty node must be evicted from the live registry"
    );
}

#[test]
fn a_fully_overlapping_nonce_reuse_with_20_round_signers_executes() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = nonce_reuse_round_pair(&p);
    let args = nonce_reuse_args(&a, &b);
    let guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);
    assert_eq!(crypto::bitmap_bits(&guilty).len(), ROUND_SIGNER_COUNT);
    let guilty_pdas = p.node_pdas_for_bitmap(args.registry_version, &guilty);

    let ix = p.punish_nonce_reuse_ix(&reporter.pubkey(), args);
    let size = p.env.transaction_size_as(&reporter.pubkey(), &[ix.clone()]);
    assert!(
        size > PACKET_DATA_SIZE,
        "20 guilty nodes exceed a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    for node in &guilty_pdas {
        let account: Node = p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
    }
    let after = p.env.registry(p.current_version());
    assert_eq!(
        after.node_count as usize,
        0,
        "every guilty node must be evicted from the live registry"
    );
}

#[test]
fn unsolicited_signing_with_20_round_signers_and_max_intruders_executes() {
    let mut p = round_protocol(UNSOLICITED_REGISTRY_NODES);
    let reporter = p.reporter();

    let (statement, intruders) = unsolicited_round_statement(&p, UNSOLICITED_REGISTRY_NODES);
    assert_eq!(intruders.len(), MAX_UNSOLICITED_INTRUDERS);
    let args = unsolicited_args(&statement);
    let guilty = unsolicited_guilty_bitmap(&p, &args);
    assert_eq!(crypto::bitmap_bits(&guilty).len(), MAX_UNSOLICITED_INTRUDERS);

    let ix = p.punish_unsolicited_signing_ix(&reporter.pubkey(), args);
    let size = p.env.transaction_size_as(&reporter.pubkey(), &[ix.clone()]);
    assert!(
        size > PACKET_DATA_SIZE,
        "a full round plus intruders exceeds a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    for bit in &intruders {
        let node = p.env.registry(statement.duty.registry_version).nodes[*bit];
        let account: Node = p.env.get(&node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
    }
    let after = p.env.registry(p.current_version());
    assert_eq!(
        after.node_count as usize,
        UNSOLICITED_REGISTRY_NODES - MAX_UNSOLICITED_INTRUDERS,
        "every intruder must be evicted from the live registry"
    );
}
