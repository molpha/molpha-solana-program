//! Reproducible CU matrix for `submit_attestation` and `submit_ticket`.
//!
//! This is deliberately a test (rather than an external script): it loads the exact SBF binary
//! built by this checkout and reports the runtime's `compute_units_consumed` field.

use molpha::instructions::SubmitAttestationArgs;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

const SIGNER_COUNTS: [usize; 3] = [8, 12, 18];

fn protocol() -> Protocol {
    let mut protocol = Protocol::builder().redundancy_buffer(1).start();
    protocol.register_nodes("submit-attestation-cu", 18);
    protocol
}

/// A larger registry lets an 18-signer bitmap fail deterministic selection. The 18-node matrix
/// above selects every registered node, so any bitmap with all 18 bits set is necessarily valid.
fn phase_protocol() -> Protocol {
    let mut protocol = Protocol::builder().redundancy_buffer(1).start();
    protocol.register_nodes("submit-attestation-cu-phases", 30);
    protocol
}

fn duty(signers: usize, timestamp: u64) -> Duty {
    // A selected coalition is quorum + redundancy_buffer (one here).
    Duty::new(18, (signers - 1) as u8)
        .source_id([signers as u8; 32])
        .at(timestamp)
}

#[test]
fn submit_attestation_cu_matrix() {
    let mut protocol = protocol();
    let submitter = protocol.env.keypair();

    for signer_count in SIGNER_COUNTS {
        let base = duty(signer_count, 1_700_000_000 + signer_count as u64 * 100);
        let signed = protocol.sign_duty(base);

        // Account parsing / Anchor constraints plus the earliest handler rejection. This uses
        // an invalid raw-value hash, so it intentionally stops before Clock and verification.
        let raw_mismatch = SubmitAttestationArgs {
            attestation: signed.attestation(),
            raw_value: Some(vec![0u8; 1]),
            coalition_key: signed.coalition_key,
        };
        let ix = protocol.submit_attestation_ix_with_args(
            &submitter.pubkey(),
            raw_mismatch,
            &signed.signer_pdas,
        );
        let early = protocol.env.send_err(&[ix], &[&submitter]);

        // Includes Clock, registry/config checks, bitmap work, but stops before the aggregate
        // verifier because the timestamp is stale.
        let stale = protocol.sign_duty(base.at(1));
        let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &stale);
        let pre_verify = protocol.env.send_err(&[ix], &[&submitter]);

        // Exercises the whole aggregate-verification path but skips feed write/event.
        let invalid_signature = signed.with_tampered_signature();
        let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &invalid_signature);
        let verify = protocol.env.send_err(&[ix], &[&submitter]);

        // First successful submission: `init_if_needed` creates the feed.
        let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &signed);
        let create = protocol.env.send_ok(&[ix], &[&submitter]);

        // Same signer set, existing feed: steady-state write path.
        let update = protocol.sign_duty(base.at(base.canonical_timestamp + 1));
        let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &update);
        let update = protocol.env.send_ok(&[ix], &[&submitter]);

        println!(
            "CU_BENCH signers={signer_count} early={} pre_verify={} verify_fail={} create={} update={}",
            early.meta.compute_units_consumed,
            pre_verify.meta.compute_units_consumed,
            verify.meta.compute_units_consumed,
            create.compute_units_consumed,
            update.compute_units_consumed,
        );
    }
}

#[test]
fn submit_attestation_18_signer_verifier_phases() {
    let mut protocol = phase_protocol();
    let submitter = protocol.env.keypair();
    let duty = Duty::new(30, 17).source_id([0x18; 32]).at(1_700_010_000);
    let signed = protocol.sign_duty(duty);
    assert_eq!(signed.signer_pdas.len(), 18);

    let stale = protocol.sign_duty(duty.at(1));
    let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &stale);
    let pre_verify = protocol.env.send_err(&[ix], &[&submitter]);

    let mut invalid_scalar = signed.clone();
    invalid_scalar.signature.agg_sig_s = [0; 32];
    let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &invalid_scalar);
    let scalar_fail = protocol.env.send_err(&[ix], &[&submitter]);

    let mut invalid_selection = signed.submit_args();
    invalid_selection.attestation.payload.canonical_timestamp += 1;
    let ix = protocol.submit_attestation_ix_with_args(
        &submitter.pubkey(),
        invalid_selection,
        &signed.signer_pdas,
    );
    let selection_fail = protocol.env.send_err(&[ix], &[&submitter]);

    let invalid_signature = signed.with_tampered_signature();
    let ix = protocol.submit_attestation_ix(&submitter.pubkey(), &invalid_signature);
    let verify_fail = protocol.env.send_err(&[ix], &[&submitter]);

    println!(
        "CU_PHASES signers=18 registry_nodes=30 pre_verify={} scalar_fail={} selection_fail={} verify_fail={}",
        pre_verify.meta.compute_units_consumed,
        scalar_fail.meta.compute_units_consumed,
        selection_fail.meta.compute_units_consumed,
        verify_fail.meta.compute_units_consumed,
    );
}

/// Same signer counts as `submit_attestation_cu_matrix`, for `submit_ticket`. Rows are the
/// whole transaction (compute budget, dispatch Ed25519 precompile, ticket). Every selected node
/// signs, so the ticket touches the whole selection. LiteSVM does not enforce the packet limit;
/// coalitions above the legacy ceiling need v0 (see `limits`).
#[test]
fn submit_ticket_cu_matrix() {
    let mut protocol = protocol();
    let gateway = protocol.register_gateway();
    let submitter = protocol.env.keypair();
    let now = protocol.env.unix_timestamp() as u64;

    for signer_count in SIGNER_COUNTS {
        let base = duty(signer_count, now + signer_count as u64);
        let signed = protocol.sign_duty(base);

        // Account constraints plus window checks, rejected by the lottery before any signature
        // work: the ticket threshold is pinned at zero.
        protocol.env.overwrite_epoch(0, |epoch| epoch.tau = 0);
        let ixs = protocol.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
        let lottery_fail = protocol.env.send_err(&ixs, &[&submitter]);
        protocol
            .env
            .overwrite_epoch(0, |epoch| epoch.tau = u64::MAX);

        // Full aggregate verification, then the invalid signature error.
        let invalid = signed.with_tampered_signature();
        let ixs = protocol.ticket_ixs(&submitter.pubkey(), &gateway, &invalid);
        let verify_fail = protocol.env.send_err(&ixs, &[&submitter]);

        // First ticket naming these nodes: roster appends with duplicate scans.
        let ixs = protocol.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
        let first = protocol.env.send_ok(&ixs, &[&submitter]);

        // Steady state: every selected node already has a roster slot.
        let next = protocol.sign_duty(base.source_id([0xee; 32]));
        let ixs = protocol.ticket_ixs(&submitter.pubkey(), &gateway, &next);
        let steady = protocol.env.send_ok(&ixs, &[&submitter]);

        println!(
            "CU_TICKET signers={signer_count} lottery_fail={} verify_fail={} first={} steady={}",
            lottery_fail.meta.compute_units_consumed,
            verify_fail.meta.compute_units_consumed,
            first.compute_units_consumed,
            steady.compute_units_consumed,
        );
    }
}
