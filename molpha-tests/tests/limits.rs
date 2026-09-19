//! How large a coalition one transaction can carry.
//!
//! Both instructions that verify an aggregate signature — `submit_attestation` and
//! `validate_round` — name one `Node` account per signer, so each signer costs 33 transaction
//! bytes (32 for the key, one for its index) and a share of the compute budget. The
//! transaction-size ceiling binds first, and by a wide margin.
//!
//! The two do not land in the same place. A production round is 18 signatures required plus a
//! 2-node redundancy buffer, so 20 signers in the derived selection. Attesting one fits a legacy
//! packet with a signer to spare; validating one does not, because validation carries the
//! settlement, gateway and token accounts a dispute needs. So the dispute path is the one that
//! *requires* a v0 transaction with an address lookup table at production round size, and the
//! attestation path only needs it past 21 signers. The last section measures and executes both.
//!
//! LiteSVM does not enforce the wire-size limit, so these tests measure the serialized
//! transaction rather than relying on a send to fail.

use anchor_lang::prelude::Pubkey;
use molpha::state::{Feed, Node, RoundReceiptRecord, RoundRecordStatus};
use molpha_tests::env::{TestEnv, PACKET_DATA_SIZE};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_message::v1;
use solana_signer::Signer;

/// Largest coalition whose `submit_attestation` transaction fits a legacy Solana packet.
///
/// Asserted exactly: adding an account to the instruction, or growing its args, costs signer
/// slots, and the gateway needs to know when that happens. Measured as the submitter would
/// really broadcast it, paying for its own single signature — the harness's own transaction
/// carries a funding payer on top, and counting that would understate the limit by three signers.
const MAX_LEGACY_COALITION: usize = 21;

/// Largest coalition whose `validate_round` transaction fits a legacy Solana packet.
///
/// One more than [`MAX_LEGACY_COALITION`]: validating carries more accounts than attesting but no
/// instruction args at all, and the args are worth more than the accounts cost.
const MAX_LEGACY_VALIDATION: usize = 19;

/// Static addresses a v1 `submit_attestation` carries before any signer: the submitter, the
/// registry, the feed, the protocol config, the system program and the Molpha program itself.
const MAX_V1_ATTESTATION_FIXED_ADDRESSES: usize = 6;

/// The same for `validate_round`, which additionally names the round, gateway, mint and the
/// token accounts a dispute pays out of. One lower than the account count suggests, because an
/// absent `subscription` compiles to the program id, which is already a key.
const MAX_V1_VALIDATION_FIXED_ADDRESSES: usize = 13;

/// Signers in a full production round: 18 signatures required plus the redundancy buffer.
///
/// Above both legacy ceilings, which is the whole reason the v0 path has to work.
const ROUND_SIGNER_COUNT: usize = 20;

const REDUNDANCY_BUFFER: u8 = 2;
/// Wide enough that the scan below runs past the wire limit rather than out of nodes.
const NODE_COUNT: usize = 30;

/// Solana's per-transaction maximum, which the harness requests for every transaction.
const MAX_COMPUTE_UNITS: u64 = 1_400_000;

fn protocol() -> Protocol {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("limits", NODE_COUNT);
    p
}

/// A duty whose derived selection has exactly `coalition` members.
fn duty_for(coalition: usize) -> Duty {
    Duty::new(
        NODE_COUNT as u32,
        (coalition - REDUNDANCY_BUFFER as usize) as u8,
    )
}

#[test]
fn the_transaction_size_limit_caps_the_coalition_before_the_compute_budget_does() {
    let mut p = protocol();
    let submitter = p.env.keypair();

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 3..=NODE_COUNT {
        let duty = duty_for(coalition).at(1_700_100_000 + coalition as u64);
        let signed = p.sign_duty(duty);
        assert_eq!(signed.signer_pdas.len(), coalition);

        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let size = p.env.transaction_size_as(&submitter.pubkey(), &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("submit_attestation: {coalition:>2} signers -> {size} transaction bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_COALITION,
        "the coalition a legacy transaction can carry changed"
    );
}

#[test]
fn submit_attestation_stays_well_inside_the_compute_budget() {
    let mut p = protocol();
    let submitter = p.env.keypair();

    let mut measurements = Vec::new();
    for coalition in [5usize, 10, 15, MAX_LEGACY_COALITION] {
        let duty = duty_for(coalition).at(1_700_000_000 + coalition as u64);
        let signed = p.sign_duty(duty);
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let meta = p.env.send_ok(&[ix], &[&submitter]);
        measurements.push((coalition, meta.compute_units_consumed));
    }

    for (coalition, consumed) in &measurements {
        println!("submit_attestation: {coalition:>2} signers -> {consumed} CU");
        assert!(
            *consumed < MAX_COMPUTE_UNITS,
            "{coalition} signers consumed {consumed} CU, over the {MAX_COMPUTE_UNITS} ceiling"
        );
    }

    // Cost grows with the coalition but is not the binding constraint.
    let (_, smallest) = measurements[0];
    let (_, largest) = measurements[measurements.len() - 1];
    assert!(largest > smallest);
    assert!(
        largest < MAX_COMPUTE_UNITS / 2,
        "a {MAX_LEGACY_COALITION}-signer round leaves no compute headroom: {largest} CU"
    );
}

#[test]
fn the_largest_transaction_sized_coalition_actually_settles() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty_for(MAX_LEGACY_COALITION));
    assert_eq!(signed.signer_pdas.len(), MAX_LEGACY_COALITION);

    let ixs = [p.submit_attestation_ix(&submitter.pubkey(), &signed)];
    assert!(p.env.transaction_size_as(&submitter.pubkey(), &ixs) <= PACKET_DATA_SIZE);
    p.env.send_ok(&ixs, &[&submitter]);
}

// ---------------------------------------------------------------------------
// validate_round
// ---------------------------------------------------------------------------

/// The same fixture with SIMD-0385 transaction v1 activated.
fn round_protocol_v1() -> (Protocol, Keypair) {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .with_env(molpha_tests::env::TestEnv::new_with_tx_v1())
        .start();
    p.register_nodes("validate-limits", NODE_COUNT);
    let gateway = p.register_gateway(5_000_000_000);
    (p, gateway)
}

/// A protocol with a funded gateway, so rounds can be settled and then disputed.
fn round_protocol() -> (Protocol, Keypair) {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("validate-limits", NODE_COUNT);
    let gateway = p.register_gateway(5_000_000_000);
    (p, gateway)
}

/// Settle an x402 round for `coalition` signers and return its record and signer accounts.
///
/// Settlement is optimistic and names no `Node` accounts at all, so it is never the instruction
/// that runs out of room — only the validation that follows it is.
fn settle(p: &mut Protocol, gateway: &Keypair, coalition: usize) -> (Pubkey, SignedAttestation) {
    let duty = duty_for(coalition).at(1_700_100_000 + coalition as u64);
    let signed = p.sign_duty(duty);
    assert_eq!(signed.signer_pdas.len(), coalition);

    let attestation = signed.attestation();
    let args = p.x402_args(&signed, [0xab; 32]);
    let record = pda::x402_round(
        &args.payer_id,
        &pda::gateway(&gateway.pubkey()),
        &attestation.payload.source_id,
        attestation.payload.signatures_required,
        attestation.payload.canonical_timestamp,
    );
    let ix = p.settle_x402_round_ix(&gateway.pubkey(), &attestation, args);
    let gw = gateway.insecure_clone();
    p.env.send_ok(&[ix], &[&gw]);
    (record, signed)
}

#[test]
fn the_transaction_size_limit_caps_a_round_validation_before_a_full_round() {
    let (mut p, gateway) = round_protocol();
    let challenger = p.challenger();

    let mut largest_fitting = 0usize;
    let mut sizes = Vec::new();
    for coalition in 5..=NODE_COUNT {
        let (record, signed) = settle(&mut p, &gateway, coalition);
        let ix = p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
        let size = p.env.transaction_size_as(&challenger.pubkey(), &[ix]);
        sizes.push((coalition, size));
        if size <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }

    for (coalition, size) in &sizes {
        println!("validate_round: {coalition:>2} signers -> {size} transaction bytes");
    }
    assert_eq!(
        largest_fitting, MAX_LEGACY_VALIDATION,
        "the coalition a legacy round validation can carry changed"
    );
    assert!(
        largest_fitting < ROUND_SIGNER_COUNT,
        "validating a full round is the case that needs a lookup table"
    );
}

#[test]
fn the_largest_transaction_sized_validation_actually_lands() {
    let (mut p, gateway) = round_protocol();
    let challenger = p.challenger();
    let (record, signed) = settle(&mut p, &gateway, MAX_LEGACY_VALIDATION);

    let ix = p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
    assert!(
        p.env
            .transaction_size_as(&challenger.pubkey(), std::slice::from_ref(&ix))
            <= PACKET_DATA_SIZE
    );
    let meta = p.env.send_ok(&[ix], &[&challenger]);
    println!(
        "validate_round: {MAX_LEGACY_VALIDATION} signers -> {} CU",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);

    // A valid signature means the round finalizes and every signer is paid.
    assert_eq!(
        p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    for node in &signed.signer_pdas {
        assert_eq!(
            p.env.get::<Node>(node).claimable_rewards,
            p.args.reward_per_signature
        );
    }
    p.assert_solvent();
}

// ---------------------------------------------------------------------------
// the v0 + address lookup table escape hatch
// ---------------------------------------------------------------------------

/// A full production round is attested inside a legacy packet, with room for one more signer.
#[test]
fn a_full_round_attestation_still_fits_a_legacy_packet() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty_for(ROUND_SIGNER_COUNT).at(1_700_150_000));
    assert_eq!(signed.signer_pdas.len(), ROUND_SIGNER_COUNT);

    let ixs = [p.submit_attestation_ix(&submitter.pubkey(), &signed)];
    let size = p.env.transaction_size_as(&submitter.pubkey(), &ixs);
    println!("submit_attestation: a full {ROUND_SIGNER_COUNT}-signer round -> {size} bytes");
    assert!(
        size <= PACKET_DATA_SIZE,
        "a full round must stay broadcastable without a lookup table ({size} bytes)"
    );
    p.env.send_ok(&ixs, &[&submitter]);
}

/// Past the legacy ceiling a lookup table is the only way to attest, and it works.
#[test]
fn a_coalition_past_the_attestation_ceiling_needs_a_v0_transaction_and_lands() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty_for(NODE_COUNT).at(1_700_150_000));
    assert_eq!(signed.signer_pdas.len(), NODE_COUNT);

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let legacy = p
        .env
        .transaction_size_as(&submitter.pubkey(), std::slice::from_ref(&ix));
    assert!(
        legacy > PACKET_DATA_SIZE,
        "{NODE_COUNT} signers should be past the legacy ceiling ({legacy} bytes)"
    );

    let lookup = TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    assert!(
        !lookup.contains(&submitter.pubkey()) && !lookup.contains(&molpha::ID),
        "the signer and the program id have to stay static"
    );
    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&submitter];

    let size = p.env.transaction_size_v0(
        &submitter.pubkey(),
        std::slice::from_ref(&ix),
        std::slice::from_ref(&table),
        &signers,
    );
    println!("submit_attestation v0+ALT: {NODE_COUNT} signers -> {size} bytes (legacy {legacy})");
    assert!(size <= PACKET_DATA_SIZE, "{size} bytes");

    let meta = p
        .env
        .send_v0_ok(&submitter.pubkey(), &[ix], &[table], &signers);
    println!(
        "submit_attestation v0+ALT: {} CU",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);

    let feed: Feed = p.env.get(&pda::feed(
        &signed.duty.source_id,
        signed.duty.signatures_required,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.canonical_timestamp, signed.duty.canonical_timestamp);
    assert_eq!(feed.value, signed.duty.value);
}

/// The same for the dispute path, which is the one that has to work when a gateway lies.
#[test]
fn a_full_round_validation_needs_a_v0_transaction_and_lands() {
    let (mut p, gateway) = round_protocol();
    let challenger = p.challenger();
    let (record, signed) = settle(&mut p, &gateway, ROUND_SIGNER_COUNT);

    let ix = p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
    let legacy = p
        .env
        .transaction_size_as(&challenger.pubkey(), std::slice::from_ref(&ix));
    assert!(legacy > PACKET_DATA_SIZE, "{legacy} bytes");

    let lookup = TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&challenger];

    let size = p.env.transaction_size_v0(
        &challenger.pubkey(),
        std::slice::from_ref(&ix),
        std::slice::from_ref(&table),
        &signers,
    );
    println!(
        "validate_round v0+ALT: {ROUND_SIGNER_COUNT} signers -> {size} bytes (legacy {legacy})"
    );
    assert!(size <= PACKET_DATA_SIZE, "{size} bytes");

    let meta = p
        .env
        .send_v0_ok(&challenger.pubkey(), &[ix], &[table], &signers);
    println!("validate_round v0+ALT: {} CU", meta.compute_units_consumed);
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);

    assert_eq!(
        p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    for node in &signed.signer_pdas {
        assert_eq!(
            p.env.get::<Node>(node).claimable_rewards,
            p.args.reward_per_signature
        );
    }
    p.assert_solvent();
}

/// With a lookup table the wire size stops being the ceiling for either instruction.
///
/// Every coalition the fixture registry can produce fits, and by a margin that grows by only two
/// bytes per signer, so what binds past here is compute rather than transaction size.
#[test]
fn a_lookup_table_removes_the_size_ceiling_for_both_verifying_instructions() {
    let (mut p, gateway) = round_protocol();
    let submitter = p.env.keypair();
    let challenger = p.challenger();

    for coalition in [MAX_LEGACY_COALITION, ROUND_SIGNER_COUNT, NODE_COUNT] {
        let (record, signed) = settle(&mut p, &gateway, coalition);

        let attest = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let table = p
            .env
            .plant_lookup_table(&TestEnv::lookup_candidates(std::slice::from_ref(&attest)));
        let attest_size = p.env.transaction_size_v0(
            &submitter.pubkey(),
            std::slice::from_ref(&attest),
            std::slice::from_ref(&table),
            &[&submitter],
        );

        let validate =
            p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
        let table = p
            .env
            .plant_lookup_table(&TestEnv::lookup_candidates(std::slice::from_ref(&validate)));
        let validate_size = p.env.transaction_size_v0(
            &challenger.pubkey(),
            std::slice::from_ref(&validate),
            std::slice::from_ref(&table),
            &[&challenger],
        );

        println!(
            "v0+ALT: {coalition:>2} signers -> submit_attestation {attest_size} bytes, \
             validate_round {validate_size} bytes"
        );
        assert!(attest_size <= PACKET_DATA_SIZE, "{attest_size} bytes");
        assert!(validate_size <= PACKET_DATA_SIZE, "{validate_size} bytes");
    }
}

// ---------------------------------------------------------------------------
// transaction v1 (SIMD-0385)
// ---------------------------------------------------------------------------

/// V1 trades lookup tables for a bigger packet and an inline compute budget.
///
/// LiteSVM accepts a v1 transaction whether or not `enable_tx_v1` is in its feature set, so these
/// numbers describe the format once SIMD-0385 activates rather than proving the gate. The fixture
/// still activates the feature so the intent survives if the harness ever starts enforcing it.
///
/// Accounts go back to being static keys at 32 bytes each, but the packet grows from 1232 to
/// [`v1::MAX_TRANSACTION_SIZE`] and the compute budget moves into the message, so there is no
/// `ComputeBudget` instruction and no `ComputeBudget` account. What limits a coalition is then
/// [`v1::MAX_ADDRESSES`], not the wire size.
#[test]
fn transaction_v1_is_bounded_by_its_address_cap_rather_than_its_packet() {
    let (mut p, gateway) = round_protocol_v1();
    let submitter = p.env.keypair();
    let challenger = p.challenger();

    let mut rows = Vec::new();
    for coalition in [
        5usize,
        MAX_LEGACY_VALIDATION,
        ROUND_SIGNER_COUNT,
        NODE_COUNT,
    ] {
        let (record, signed) = settle(&mut p, &gateway, coalition);

        let attest = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let attest_size = p.env.transaction_size_v1(
            &submitter.pubkey(),
            std::slice::from_ref(&attest),
            &[&submitter],
        );
        let attest_keys = p
            .env
            .v1_address_count(&submitter.pubkey(), std::slice::from_ref(&attest));

        let validate =
            p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
        let validate_size = p.env.transaction_size_v1(
            &challenger.pubkey(),
            std::slice::from_ref(&validate),
            &[&challenger],
        );
        let validate_keys = p
            .env
            .v1_address_count(&challenger.pubkey(), std::slice::from_ref(&validate));

        println!(
            "v1: {coalition:>2} signers -> submit_attestation {attest_size} bytes / \
             {attest_keys} addresses, validate_round {validate_size} bytes / {validate_keys} addresses"
        );
        rows.push((
            coalition,
            attest_size,
            attest_keys,
            validate_size,
            validate_keys,
        ));
    }

    // Nothing comes close to the packet limit, so the address cap is what binds.
    for (coalition, attest_size, attest_keys, validate_size, validate_keys) in &rows {
        assert!(
            *attest_size < v1::MAX_TRANSACTION_SIZE && *validate_size < v1::MAX_TRANSACTION_SIZE,
            "{coalition} signers should be nowhere near the v1 packet limit"
        );
        assert!(*attest_keys <= usize::from(v1::MAX_ADDRESSES));
        assert!(*validate_keys <= usize::from(v1::MAX_ADDRESSES));
    }

    // Each extra signer costs exactly one address, so the ceiling follows from the fixed cost.
    let (_, _, keys_at_20, _, validate_keys_at_20) = rows[2];
    let attest_fixed = keys_at_20 - ROUND_SIGNER_COUNT;
    let validate_fixed = validate_keys_at_20 - ROUND_SIGNER_COUNT;
    println!(
        "v1 ceilings: submit_attestation {} signers, validate_round {} signers",
        usize::from(v1::MAX_ADDRESSES) - attest_fixed,
        usize::from(v1::MAX_ADDRESSES) - validate_fixed
    );
    assert_eq!(attest_fixed, MAX_V1_ATTESTATION_FIXED_ADDRESSES);
    assert_eq!(validate_fixed, MAX_V1_VALIDATION_FIXED_ADDRESSES);
}

/// A full production round needs no lookup table under v1 — one transaction, no setup.
#[test]
fn a_full_round_attests_and_validates_in_a_single_v1_transaction() {
    let (mut p, gateway) = round_protocol_v1();
    let submitter = p.env.keypair();
    let challenger = p.challenger();
    let (record, signed) = settle(&mut p, &gateway, ROUND_SIGNER_COUNT);

    let attest = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let meta = p
        .env
        .send_v1_ok(&submitter.pubkey(), &[attest], &[&submitter]);
    println!(
        "submit_attestation v1: {ROUND_SIGNER_COUNT} signers -> {} CU",
        meta.compute_units_consumed
    );
    let feed: Feed = p.env.get(&pda::feed(
        &signed.duty.source_id,
        signed.duty.signatures_required,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.canonical_timestamp, signed.duty.canonical_timestamp);

    let validate = p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
    let meta = p
        .env
        .send_v1_ok(&challenger.pubkey(), &[validate], &[&challenger]);
    println!(
        "validate_round v1: {ROUND_SIGNER_COUNT} signers -> {} CU",
        meta.compute_units_consumed
    );
    assert_eq!(
        p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    for node in &signed.signer_pdas {
        assert_eq!(
            p.env.get::<Node>(node).claimable_rewards,
            p.args.reward_per_signature
        );
    }
    p.assert_solvent();
}
