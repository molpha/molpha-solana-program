//! Transaction-size and compute limits for aggregate verification.

use molpha_tests::env::PACKET_DATA_SIZE;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

/// Largest coalition whose `submit_attestation` fits a legacy packet. The 64-byte coalition key
/// in instruction data costs two 32-byte signer accounts (was 21 without it); v0 transactions
/// with a lookup table are unaffected.
const MAX_LEGACY_COALITION: usize = 19;
const REDUNDANCY_BUFFER: u8 = 2;
const NODE_COUNT: usize = 30;
const MAX_COMPUTE_UNITS: u64 = 1_400_000;

fn protocol() -> Protocol {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("limits", NODE_COUNT);
    p
}

fn duty_for(coalition: usize) -> Duty {
    Duty::new(
        NODE_COUNT as u32,
        (coalition - REDUNDANCY_BUFFER as usize) as u8,
    )
}

#[test]
fn submit_attestation_legacy_size_ceiling_is_pinned() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    let mut largest_fitting = 0usize;
    for coalition in 3..=NODE_COUNT {
        let signed = p.sign_duty(duty_for(coalition).at_secs(1_700_100_000 + coalition as u64));
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        if p.env.transaction_size_as(&submitter.pubkey(), &[ix]) <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }
    assert_eq!(largest_fitting, MAX_LEGACY_COALITION);
}

#[test]
fn submit_attestation_stays_inside_the_compute_budget() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    for coalition in [5usize, 10, 15, MAX_LEGACY_COALITION] {
        let signed = p.sign_duty(duty_for(coalition).at_secs(1_700_000_000 + coalition as u64));
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let meta = p.env.send_ok(&[ix], &[&submitter]);
        assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
    }
}

/// Largest coalition whose `[dispatch Ed25519, submit_ticket]` pair fits a legacy packet with
/// the compute-budget instruction. Larger coalitions need a v0 transaction with a lookup table.
const MAX_LEGACY_TICKET_COALITION: usize = 10;

/// A ticket duty at the start of the open epoch, distinct per coalition size.
fn ticket_duty_for(p: &Protocol, coalition: usize) -> Duty {
    duty_for(coalition).at_secs(p.env.unix_timestamp() as u64 + coalition as u64)
}

#[test]
fn submit_ticket_legacy_size_ceiling_is_pinned() {
    let mut p = protocol();
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let mut largest_fitting = 0usize;
    for coalition in 3..=NODE_COUNT {
        let signed = p.sign_duty(ticket_duty_for(&p, coalition));
        let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
        if p.env.transaction_size_as(&submitter.pubkey(), &ixs) <= PACKET_DATA_SIZE {
            largest_fitting = coalition;
        }
    }
    assert_eq!(largest_fitting, MAX_LEGACY_TICKET_COALITION);
}

/// A 20-signer ticket (quorum 18, buffer 2) only fits with the signer accounts in a lookup table;
/// the marker, epoch and payer stay static.
#[test]
fn a_twenty_signer_ticket_fits_v0_and_the_compute_budget() {
    let mut p = protocol();
    let gateway = p.register_gateway();
    let signed = p.sign_duty(ticket_duty_for(&p, 20));
    let ixs = p.ticket_ixs(&gateway.pubkey(), &gateway, &signed);
    let lookup: Vec<_> = signed.signer_pdas.clone();
    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&gateway];
    let size = p.env.transaction_size_v0(
        &gateway.pubkey(),
        &ixs,
        std::slice::from_ref(&table),
        &signers,
    );
    println!("TICKET_V0 signers=20 size={size}");
    assert!(size <= PACKET_DATA_SIZE);
    let meta = p
        .env
        .send_v0_ok(&gateway.pubkey(), &ixs, &[table], &signers);
    println!("TICKET_V0 signers=20 cu={}", meta.compute_units_consumed);
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
}
