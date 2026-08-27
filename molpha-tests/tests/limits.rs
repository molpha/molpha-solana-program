//! How large a coalition one transaction can carry.
//!
//! Every signer costs a `remaining_accounts` slot (33 bytes of transaction, including its
//! metadata byte) and a share of the compute budget, so quorum size is bounded by the runtime,
//! not by the program. Of the two ceilings the transaction-size one binds first, and by a wide
//! margin — the compute budget is nowhere near exhausted at the largest coalition that fits.
//!
//! LiteSVM does not enforce the wire-size limit, so these tests measure the serialized
//! transaction rather than relying on a send to fail.

use molpha_tests::env::PACKET_DATA_SIZE;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

/// Largest coalition whose `submit_attestation` transaction fits a legacy Solana packet.
///
/// Asserted exactly: adding an account to the instruction, or growing its args, costs signer
/// slots, and the gateway needs to know when that happens.
const MAX_LEGACY_COALITION: usize = 19;

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
        let size = p.env.transaction_size(&[ix], &[&submitter]);
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
    assert!(p.env.transaction_size(&ixs, &[&submitter]) <= PACKET_DATA_SIZE);
    p.env.send_ok(&ixs, &[&submitter]);
}
