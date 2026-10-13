//! Transaction-size and compute limits for aggregate verification.
//!
//! Signer keys come from the registry snapshot, so neither instruction carries a per-signer
//! account. What still grows with the coalition is compute (about 43k + 9k units per signer) and,
//! for a ticket, two bytes of roster-slot hint per selected node.

use molpha_tests::env::PACKET_DATA_SIZE;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

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

/// No per-signer accounts: the size of a submission is the same for every coalition, and fits a
/// legacy packet.
#[test]
fn submit_attestation_size_is_independent_of_the_coalition() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    let mut sizes = Vec::new();
    for coalition in 3..=NODE_COUNT {
        let signed = p.sign_duty(duty_for(coalition).at_secs(1_700_100_000 + coalition as u64));
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        sizes.push(p.env.transaction_size_as(&submitter.pubkey(), &[ix]));
    }
    println!("ATTESTATION_SIZE bytes={}", sizes[0]);
    assert!(sizes.iter().all(|size| *size == sizes[0]), "{sizes:?}");
    assert!(sizes[0] <= PACKET_DATA_SIZE);
}

/// Signer keys cost compute, not accounts, so a large coalition is bounded by the transaction's
/// compute budget rather than by its size. `registry_allocation` pins where that bound falls.
const WIDE_COALITION: usize = 96;

#[test]
fn a_96_signer_attestation_stays_inside_the_compute_budget() {
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    p.register_nodes("limits-wide-coalition", WIDE_COALITION);
    let submitter = p.env.keypair();
    let quorum = (WIDE_COALITION - REDUNDANCY_BUFFER as usize) as u8;
    let signed = p.sign_duty(Duty::new(WIDE_COALITION as u32, quorum).at_secs(1_700_200_000));
    assert_eq!(signed.signer_pdas.len(), WIDE_COALITION);

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let meta = p.env.send_ok(&[ix], &[&submitter]);
    println!(
        "ATTESTATION_CU coalition={WIDE_COALITION} cu={}",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
}

#[test]
fn submit_attestation_stays_inside_the_compute_budget() {
    let mut p = protocol();
    let submitter = p.env.keypair();
    for coalition in [5usize, 10, 15, 20, 25, NODE_COUNT] {
        let signed = p.sign_duty(duty_for(coalition).at_secs(1_700_000_000 + coalition as u64));
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let meta = p.env.send_ok(&[ix], &[&submitter]);
        println!(
            "ATTESTATION_CU coalition={coalition} cu={}",
            meta.compute_units_consumed
        );
        assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
    }
}

/// A ticket duty at the start of the open epoch, distinct per coalition size.
fn ticket_duty_for(p: &Protocol, coalition: usize) -> Duty {
    duty_for(coalition).at_secs(p.env.unix_timestamp() as u64 + coalition as u64)
}

/// A ticket's size grows two bytes per selected node (its roster-slot hint) and nothing else, so
/// a coalition larger than compute allows still fits a legacy packet with the dispatch
/// instruction.
#[test]
fn submit_ticket_size_grows_two_bytes_per_signer_and_fits_a_legacy_packet() {
    let mut p = protocol();
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let mut sizes = Vec::new();
    for coalition in 3..=NODE_COUNT {
        let signed = p.sign_duty(ticket_duty_for(&p, coalition));
        let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
        sizes.push((
            coalition,
            p.env.transaction_size_as(&submitter.pubkey(), &ixs),
        ));
    }
    let (first, base) = sizes[0];
    for (coalition, size) in &sizes {
        assert_eq!(
            *size,
            base + 2 * (coalition - first),
            "coalition {coalition}"
        );
    }
    println!("TICKET_SIZE base={base} per_signer=2");
    // Extrapolated past the largest coalition the compute budget admits (about 155 signers).
    assert!(base + 2 * (160 - first) <= PACKET_DATA_SIZE);
}

/// A 20-signer ticket (quorum 18, buffer 2) needs no lookup table: the signers are not accounts.
#[test]
fn a_twenty_signer_ticket_fits_a_legacy_packet_and_the_compute_budget() {
    let mut p = protocol();
    let gateway = p.register_gateway();
    let signed = p.sign_duty(ticket_duty_for(&p, 20));
    let ixs = p.ticket_ixs(&gateway.pubkey(), &gateway, &signed);
    let size = p.env.transaction_size_as(&gateway.pubkey(), &ixs);
    println!("TICKET signers=20 size={size}");
    assert!(size <= PACKET_DATA_SIZE);
    let meta = p.env.send_ok(&ixs, &[&gateway]);
    println!("TICKET signers=20 cu={}", meta.compute_units_consumed);
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
}
