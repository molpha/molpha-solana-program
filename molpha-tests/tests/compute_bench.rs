//! Reproducible CU matrix for `submit_attestation` and both settlement instructions.
//!
//! This is deliberately a test (rather than an external script): it loads the exact SBF binary
//! built by this checkout and reports the runtime's `compute_units_consumed` field.

use anchor_lang::solana_program::instruction::Instruction;
use molpha::instructions::SubmitAttestationArgs;
use molpha::state::{PlanConfig, PlanType};
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
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

/// Nodes, a plan that admits the 17-of-18 quorum, one subscriber, and a funded gateway.
fn settlement_protocol() -> (Protocol, Keypair, Keypair) {
    let mut protocol = Protocol::builder().redundancy_buffer(1).start();
    protocol.register_nodes("settlement-cu", 18);
    protocol.add_plan(PlanConfig {
        plan_type: PlanType::Professional,
        subscription_price: 50_000_000,
        // `signatures_required` is signer_count - 1, so the 18-signer row asks for 17.
        max_signers: 18,
        max_delegates: 1,
        max_rounds: 16,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = protocol.subscribe(PlanType::Professional);
    let gateway = protocol.register_gateway(50_000_000);
    (protocol, owner, gateway)
}

/// `[ed25519 consumer authorization, settle_subscription_round]`.
fn subscription_ixs(
    protocol: &Protocol,
    gateway: &Keypair,
    owner: &Keypair,
    auth: &Duty,
    signed: &SignedAttestation,
) -> Vec<Instruction> {
    vec![
        protocol.consumer_authorization_ix(owner, &gateway.pubkey(), auth),
        protocol.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            signed,
        ),
    ]
}

/// Same signer counts as `submit_attestation_cu_matrix`, for both settlement instructions.
///
/// Every selected node signs, so remaining accounts are exactly that coalition (no missed-node
/// liveliness tail). Subscription rows are the whole transaction, Ed25519 authorization included.
/// x402 is one instruction. Rejecting rows still pay for Anchor's `init` of the replay marker;
/// the allocation is rolled back with the error.
#[test]
fn settlement_cu_matrix() {
    let (mut protocol, owner, gateway) = settlement_protocol();
    let owner_signer = owner.insecure_clone();
    let gateway_signer = gateway.insecure_clone();

    for signer_count in SIGNER_COUNTS {
        let base = duty(signer_count, 1_700_000_000 + signer_count as u64 * 100);
        let signed = protocol.sign_duty(base);
        let payers = [&owner_signer, &gateway_signer];

        // Valid precompile over a different request: account setup plus the Ed25519 check, and
        // nothing from `validate_attestation`.
        let early_auth = base.at(base.canonical_timestamp + 50);
        let ixs = subscription_ixs(&protocol, &gateway, &owner, &early_auth, &signed);
        let sub_early = protocol.env.send_err(&ixs, &payers);

        // Clock, quota, consumer auth, registry and bitmap checks. Stops before the aggregate
        // verifier because the timestamp is stale.
        let stale = protocol.sign_duty(base.at(1));
        let ixs = subscription_ixs(&protocol, &gateway, &owner, &stale.duty, &stale);
        let sub_pre_verify = protocol.env.send_err(&ixs, &payers);

        // Full aggregate verification, then the error before rewards are credited.
        let invalid = signed.with_tampered_signature();
        let ixs = subscription_ixs(&protocol, &gateway, &owner, &base, &invalid);
        let sub_verify = protocol.env.send_err(&ixs, &payers);

        let ixs = subscription_ixs(&protocol, &gateway, &owner, &base, &signed);
        let sub_settle = protocol.env.send_ok(&ixs, &payers);

        let payer_id = [signer_count as u8; 32];
        let mut underpaid = protocol.x402_args(&signed, payer_id);
        underpaid.amount -= 1;
        // Price check runs before `validate_attestation`.
        let ix = protocol.settle_x402_round_ix(&gateway.pubkey(), &signed.attestation(), underpaid);
        let x402_early = protocol.env.send_err(&[ix], &[&gateway_signer]);

        let ix = protocol.settle_x402_round_ix(
            &gateway.pubkey(),
            &stale.attestation(),
            protocol.x402_args(&stale, payer_id),
        );
        let x402_pre_verify = protocol.env.send_err(&[ix], &[&gateway_signer]);

        let ix = protocol.settle_x402_round_ix(
            &gateway.pubkey(),
            &invalid.attestation(),
            protocol.x402_args(&signed, payer_id),
        );
        let x402_verify = protocol.env.send_err(&[ix], &[&gateway_signer]);

        let ix = protocol.settle_x402_round_ix(
            &gateway.pubkey(),
            &signed.attestation(),
            protocol.x402_args(&signed, payer_id),
        );
        let x402_settle = protocol.env.send_ok(&[ix], &[&gateway_signer]);

        println!(
            "CU_SETTLE path=subscription signers={signer_count} early={} pre_verify={} verify_fail={} settle={}",
            sub_early.meta.compute_units_consumed,
            sub_pre_verify.meta.compute_units_consumed,
            sub_verify.meta.compute_units_consumed,
            sub_settle.compute_units_consumed,
        );
        println!(
            "CU_SETTLE path=x402 signers={signer_count} early={} pre_verify={} verify_fail={} settle={}",
            x402_early.meta.compute_units_consumed,
            x402_pre_verify.meta.compute_units_consumed,
            x402_verify.meta.compute_units_consumed,
            x402_settle.compute_units_consumed,
        );
    }
}
