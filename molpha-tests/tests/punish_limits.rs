//! How large a guilty set one punishment transaction can carry.
//!
//! A punishment names one `Node` account per signer and nothing else that scales: the guilty
//! nodes' principal all sits in one vault, so the sentence settles in a single transfer whatever
//! the size of the guilty set. Each signer still costs 33 transaction bytes, so reporters need to
//! know where the limit lands: past it, a filing is only broadcastable as a v0 transaction with
//! an address lookup table.
//!
//! LiteSVM does not enforce the wire limit, so these tests measure rather than expecting a
//! failed send, and they measure with the reporter as fee payer — the way a filing is really sent.
//!
//! The v0 tests at the end close the loop: they resolve the same filing through an address
//! lookup table and send it, proving the escape hatch is real rather than assumed.
//!
//! The production round shape we must remain able to punish is 18 signatures required plus 2
//! redundancy buffer — 20 signers in the derived selection. Legacy wire limits only allow filing
//! a handful of guilty nodes without a v0 transaction; the execution tests below are the guard
//! that a full 20-node coalition still lands on-chain (unsolicited signing tests the same round
//! shape with the maximum intruder count that fits one transaction).

use molpha::state::{
    CommitmentReuseEvidence, EquivocationEvidence, Node, NodeStatus, UnsolicitedSigningEvidence,
};
use molpha_tests::crypto;
use molpha_tests::env::{TestEnv, PACKET_DATA_SIZE};
use molpha_tests::scenario::{
    intersect_bitmap, unsolicited_guilty_bitmap, Duty, Protocol, SignedAttestation,
};
use solana_signer::Signer;

/// Registry size for legacy wire-size sweeps — wide enough for coalition probes up to 12.
const NODE_COUNT: usize = 30;
const REDUNDANCY_BUFFER: u8 = 2;

/// Round signer set: 18 threshold + 2 redundancy buffer.
const SIGS_REQUIRED: u8 = 18;
const ROUND_SIGNER_COUNT: usize = SIGS_REQUIRED as usize + REDUNDANCY_BUFFER as usize;

/// Intruders punishable in one unsolicited filing alongside a 20-signer round attestation.
///
/// Fixed accounts plus remaining `Node` accounts for the full submitted bitmap (20 round signers
/// + N intruders) must stay within runtime account-lock limits in LiteSVM as well as the
/// 64-account wire cap.
const MAX_UNSOLICITED_INTRUDERS: usize = 8;
const UNSOLICITED_REGISTRY_NODES: usize = ROUND_SIGNER_COUNT + MAX_UNSOLICITED_INTRUDERS;

/// Largest fully-overlapping coalition whose `slash_offence` (equivocation) filing fits a legacy
/// packet.
///
/// Asserted exactly. A filing costs one account per signer plus a fixed set of token accounts,
/// so every account added to the punishment context costs a signer directly, and going back to a
/// vault per guilty node would halve this number. Both attestations carry their own duty fields,
/// so the args are as large as commitment reuse and the limit matches it.
const MAX_LEGACY_EQUIVOCATION: usize = 10;

/// Same ceiling for `slash_offence` (commitment reuse), whose two full attestations make its args
/// the largest pair-offence encoding.
const MAX_LEGACY_COMMITMENT_REUSE: usize = 10;

/// Solana's per-transaction maximum, which the harness requests for every transaction.
const MAX_COMPUTE_UNITS: u64 = 1_400_000;

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

fn equivocation_args(a: &SignedAttestation, b: &SignedAttestation) -> EquivocationEvidence {
    let (attestation_a, attestation_b) = (a.attestation(), b.attestation());
    EquivocationEvidence {
        attestation_a,
        attestation_b,
    }
}

/// Fully overlapping commitment reuse over the full round signer set — two duties with the same
/// `source_id` but different timestamps whose selections intersect on `ROUND_SIGNER_COUNT` bits.
fn commitment_reuse_round_pair(p: &Protocol) -> (SignedAttestation, SignedAttestation) {
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

fn commitment_reuse_args(a: &SignedAttestation, b: &SignedAttestation) -> CommitmentReuseEvidence {
    let (attestation_a, attestation_b) = (a.attestation(), b.attestation());
    CommitmentReuseEvidence {
        attestation_a,
        attestation_b,
    }
}

fn unsolicited_round_attestation(
    p: &Protocol,
    node_count: usize,
) -> (SignedAttestation, Vec<usize>) {
    let duty = Duty::new(p.current_version(), SIGS_REQUIRED);
    let selection = p.selection_bits(&duty);
    assert_round_selection(p, &duty);
    let intruders: Vec<usize> = (0..node_count)
        .filter(|bit| !selection.contains(bit))
        .collect();
    let mut bits = selection;
    bits.extend(&intruders);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(duty, &bits, None);
    (signed, intruders)
}

fn unsolicited_args(signed: &SignedAttestation) -> UnsolicitedSigningEvidence {
    let attestation = signed.attestation();
    UnsolicitedSigningEvidence { attestation }
}

#[test]
fn a_fully_overlapping_equivocation_fits_a_legacy_packet_up_to_the_pinned_coalition() {
    let mut p = protocol();
    let reporter = p.reporter().pubkey();

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 3..=20 {
        let (a, b) = equivocation_pair(&p, coalition, 1_700_100_000 + coalition as u64);
        let args = equivocation_args(&a, &b);
        let ix = p.slash_equivocation_ix(&reporter, args);
        let size = p.env.transaction_size_as(&reporter, &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("slash_equivocation: {coalition:>2} signers, all guilty -> {size} bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_EQUIVOCATION,
        "the guilty set a legacy equivocation filing can carry changed"
    );
}

/// Size is only the first ceiling. A filing verifies two aggregate signatures over the whole
/// coalition and moves money twice, so the compute budget is what to watch next.
#[test]
fn the_largest_fitting_equivocation_filing_actually_lands() {
    let mut p = protocol();
    let reporter = p.reporter();

    let (a, b) = equivocation_pair(&p, MAX_LEGACY_EQUIVOCATION, 1_700_200_500);
    let args = equivocation_args(&a, &b);
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), args);
    assert!(
        p.env
            .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix))
            <= PACKET_DATA_SIZE
    );

    let meta = p.env.send_ok(&[ix], &[&reporter]);
    println!(
        "slash_equivocation: {MAX_LEGACY_EQUIVOCATION} signers, all guilty -> {} CU",
        meta.compute_units_consumed
    );
    assert!(
        meta.compute_units_consumed < MAX_COMPUTE_UNITS,
        "the largest transaction-sized filing does not fit the compute budget: {} CU",
        meta.compute_units_consumed
    );
}

#[test]
fn a_fully_overlapping_commitment_reuse_fits_a_legacy_packet_up_to_the_pinned_coalition() {
    let mut p = protocol();
    let reporter = p.reporter().pubkey();
    const SHARED_NONCE: [u8; 32] = [0x5c; 32];

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 3..=20 {
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
        let args = commitment_reuse_args(&a, &b);
        let ix = p.slash_commitment_reuse_ix(&reporter, args);
        let size = p.env.transaction_size_as(&reporter, &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("slash_commitment_reuse: {coalition:>2} signers, all guilty -> {size} bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_COMMITMENT_REUSE,
        "the guilty set a legacy commitment-reuse filing can carry changed"
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
    let guilty_pdas = p.node_pdas_for_bitmap(a.duty.registry_version, &guilty);

    let ix = p.slash_equivocation_ix(&reporter.pubkey(), args);
    let size = p
        .env
        .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix));
    assert!(
        size > PACKET_DATA_SIZE,
        "20 guilty nodes exceed a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    let meta = p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    println!(
        "{ROUND_SIGNER_COUNT} signers, all guilty -> {size} bytes, {} CU",
        meta.compute_units_consumed
    );
    for node in &guilty_pdas {
        let account: Node = p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
    }
    let after = p.env.registry(p.current_version());
    assert_eq!(
        after.node_count as usize, 0,
        "every guilty node must be evicted from the live registry"
    );
}

#[test]
fn a_fully_overlapping_commitment_reuse_with_20_round_signers_executes() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = commitment_reuse_round_pair(&p);
    let args = commitment_reuse_args(&a, &b);
    let guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);
    assert_eq!(crypto::bitmap_bits(&guilty).len(), ROUND_SIGNER_COUNT);
    let guilty_pdas = p.node_pdas_for_bitmap(a.duty.registry_version, &guilty);

    let ix = p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let size = p
        .env
        .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix));
    assert!(
        size > PACKET_DATA_SIZE,
        "20 guilty nodes exceed a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    let meta = p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    println!(
        "{ROUND_SIGNER_COUNT} signers, all guilty -> {size} bytes, {} CU",
        meta.compute_units_consumed
    );
    for node in &guilty_pdas {
        let account: Node = p.env.get(node);
        assert_eq!(account.status, NodeStatus::Tombstoned);
    }
    let after = p.env.registry(p.current_version());
    assert_eq!(
        after.node_count as usize, 0,
        "every guilty node must be evicted from the live registry"
    );
}

#[test]
fn unsolicited_signing_with_20_round_signers_and_max_intruders_executes() {
    let mut p = round_protocol(UNSOLICITED_REGISTRY_NODES);
    let reporter = p.reporter();

    let (signed, intruders) = unsolicited_round_attestation(&p, UNSOLICITED_REGISTRY_NODES);
    assert_eq!(intruders.len(), MAX_UNSOLICITED_INTRUDERS);
    let args = unsolicited_args(&signed);
    let guilty = unsolicited_guilty_bitmap(&p, &args.attestation);
    assert_eq!(
        crypto::bitmap_bits(&guilty).len(),
        MAX_UNSOLICITED_INTRUDERS
    );

    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), args);
    let size = p
        .env
        .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix));
    assert!(
        size > PACKET_DATA_SIZE,
        "a full round plus intruders exceeds a legacy packet ({size} bytes); v0+ALT is required on mainnet"
    );

    let meta = p.env.send_ok_with_max_heap(&[ix], &[&reporter]);
    println!(
        "{ROUND_SIGNER_COUNT} signers, all guilty -> {size} bytes, {} CU",
        meta.compute_units_consumed
    );
    for bit in &intruders {
        let node = p.env.registry(signed.duty.registry_version).nodes[*bit];
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

// ---------------------------------------------------------------------------
// the v0 + address lookup table escape hatch
// ---------------------------------------------------------------------------

/// Everything a filing names except its signer and the programs it calls.
///
/// Pinned as a count so that adding an account to a punishment shows up here too: a lookup table
/// absorbs it, but it still costs a byte and still has to be in the table.
const EQUIVOCATION_LOOKUP_ACCOUNTS: usize = 10;

/// A full production round is 18 signatures required plus a 2-node buffer. Punishing all 20 does
/// not fit a legacy packet, and this is the transaction a reporter would actually broadcast.
#[test]
fn a_twenty_signer_equivocation_fits_a_packet_as_a_v0_transaction_and_lands() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = equivocation_pair(&p, ROUND_SIGNER_COUNT, 1_700_300_000);
    assert_round_selection(&p, &a.duty);
    let args = equivocation_args(&a, &b);
    let guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);
    let guilty_pdas = p.node_pdas_for_bitmap(a.duty.registry_version, &guilty);
    assert_eq!(guilty_pdas.len(), ROUND_SIGNER_COUNT);

    let ix = p.slash_equivocation_ix(&reporter.pubkey(), args);
    let legacy = p
        .env
        .transaction_size_as(&reporter.pubkey(), std::slice::from_ref(&ix));
    assert!(
        legacy > PACKET_DATA_SIZE,
        "this case is only interesting because it does not fit a legacy packet"
    );

    // Signers and program ids have to stay in the static keys; everything else can be looked up.
    let lookup = TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    assert_eq!(
        lookup.len(),
        EQUIVOCATION_LOOKUP_ACCOUNTS + ROUND_SIGNER_COUNT,
        "the fixed account list changed"
    );
    assert!(!lookup.contains(&reporter.pubkey()), "the signer is static");
    assert!(!lookup.contains(&molpha::ID), "the program id is static");

    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&reporter];
    let size = p.env.transaction_size_v0(
        &reporter.pubkey(),
        std::slice::from_ref(&ix),
        std::slice::from_ref(&table),
        &signers,
    );
    println!(
        "slash_equivocation v0+ALT: {ROUND_SIGNER_COUNT} signers, all guilty -> \
         {size} bytes (legacy {legacy})"
    );
    assert!(
        size <= PACKET_DATA_SIZE,
        "a lookup table has to bring a 20-signer filing back inside a packet ({size} bytes)"
    );

    let meta = p
        .env
        .send_v0_ok(&reporter.pubkey(), &[ix], &[table], &signers);
    println!(
        "slash_equivocation v0+ALT: {} CU",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);

    // It is a real filing, not just a well-formed transaction.
    for node in &guilty_pdas {
        assert_eq!(p.env.get::<Node>(node).status, NodeStatus::Tombstoned);
        assert_eq!(p.env.get::<Node>(node).locked_amount, 0);
    }
    assert_eq!(
        p.env.registry(p.current_version()).node_count as usize,
        0,
        "every guilty node is evicted"
    );
    p.assert_solvent();
}

/// Nonce reuse carries the largest args of the three, so it is the tightest v0 case.
#[test]
fn a_twenty_signer_commitment_reuse_fits_a_packet_as_a_v0_transaction_and_lands() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = commitment_reuse_round_pair(&p);
    let args = commitment_reuse_args(&a, &b);
    let guilty = intersect_bitmap(&a.signers_bitmap, &b.signers_bitmap);
    let guilty_pdas = p.node_pdas_for_bitmap(a.duty.registry_version, &guilty);
    assert_eq!(guilty_pdas.len(), ROUND_SIGNER_COUNT);

    let ix = p.slash_commitment_reuse_ix(&reporter.pubkey(), args);
    let lookup = TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&reporter];

    let size = p.env.transaction_size_v0(
        &reporter.pubkey(),
        std::slice::from_ref(&ix),
        std::slice::from_ref(&table),
        &signers,
    );
    println!(
        "slash_commitment_reuse v0+ALT: {ROUND_SIGNER_COUNT} signers, all guilty -> {size} bytes"
    );
    assert!(size <= PACKET_DATA_SIZE, "{size} bytes");

    let meta = p
        .env
        .send_v0_ok(&reporter.pubkey(), &[ix], &[table], &signers);
    println!(
        "slash_commitment_reuse v0+ALT: {} CU",
        meta.compute_units_consumed
    );

    for node in &guilty_pdas {
        assert_eq!(p.env.get::<Node>(node).status, NodeStatus::Tombstoned);
    }
    p.assert_solvent();
}

/// The coalition is the part that has to be in the table.
///
/// A table holding only the filing's fixed accounts still leaves it over the limit, so tooling
/// that builds a table from the instruction's static context and forgets `remaining_accounts`
/// would produce a transaction that cannot be broadcast.
#[test]
fn a_lookup_table_without_the_coalition_does_not_bring_the_filing_under_the_limit() {
    let mut p = round_protocol(ROUND_SIGNER_COUNT);
    let reporter = p.reporter();

    let (a, b) = equivocation_pair(&p, ROUND_SIGNER_COUNT, 1_700_300_000);
    let args = equivocation_args(&a, &b);
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), args);

    // Everything except the 20 signer `Node` accounts, which sit at the tail.
    let full = TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    let fixed_only = &full[..EQUIVOCATION_LOOKUP_ACCOUNTS];
    let table = p.env.plant_lookup_table(fixed_only);
    let signers = [&reporter];

    let size = p.env.transaction_size_v0(
        &reporter.pubkey(),
        std::slice::from_ref(&ix),
        &[table],
        &signers,
    );
    println!("slash_equivocation v0, fixed accounts only -> {size} bytes");
    assert!(
        size > PACKET_DATA_SIZE,
        "the coalition has to be in the table ({size} bytes)"
    );
}
