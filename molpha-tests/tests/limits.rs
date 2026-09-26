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
        let signed = p.sign_duty(duty_for(coalition).at(1_700_100_000 + coalition as u64));
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
        let signed = p.sign_duty(duty_for(coalition).at(1_700_000_000 + coalition as u64));
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let meta = p.env.send_ok(&[ix], &[&submitter]);
        assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
    }
}

#[test]
fn immediate_settlement_names_the_whole_selection_and_fits_v0() {
    let mut p = protocol();
    let gateway = p.register_gateway(5_000_000_000);
    let signed = p.sign_duty(duty_for(20).at(1_700_200_000));
    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &signed.attestation(),
        p.x402_args(&signed, [0xaa; 32]),
    );
    let lookup = molpha_tests::env::TestEnv::lookup_candidates(std::slice::from_ref(&ix));
    let table = p.env.plant_lookup_table(&lookup);
    let signers = [&gateway];
    assert!(
        p.env.transaction_size_v0(
            &gateway.pubkey(),
            std::slice::from_ref(&ix),
            std::slice::from_ref(&table),
            &signers,
        ) <= PACKET_DATA_SIZE
    );
    let meta = p
        .env
        .send_v0_ok(&gateway.pubkey(), &[ix], &[table], &signers);
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
}
