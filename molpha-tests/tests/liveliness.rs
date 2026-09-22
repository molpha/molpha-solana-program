//! Immediate liveliness updates during round settlement.

use molpha::error::MolphaError;
use molpha::state::{Node, LIVELINESS_SCORE_MAX};
use molpha_tests::env::assert_molpha_error;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

const NODE_COUNT: usize = 8;

#[test]
fn settlement_rewards_signers_and_penalizes_selected_misses() {
    let mut p = Protocol::builder()
        .liveliness_penalty_per_miss(750)
        .liveliness_recovery_per_sign(225)
        .start();
    p.register_nodes("liveliness-immediate", NODE_COUNT);
    let gateway = p.register_gateway(500_000_000);
    let duty = Duty::new(NODE_COUNT as u32, 4);
    let selected = p.selection_bits(&duty);
    let signer_bits = &selected[..4];
    let signed = p.sign_duty_with_bits(duty, signer_bits, None);

    let recovering = signed.signer_pdas[0];
    p.overwrite_node(&recovering, |node| node.liveliness_score = 1_000);
    let missed: Vec<_> = selected[4..]
        .iter()
        .map(|bit| p.env.registry(duty.registry_version).nodes[*bit])
        .collect();

    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &signed.attestation(),
        p.x402_args(&signed, [0xaa; 32]),
    );
    p.env.send_ok(&[ix], &[&gateway]);

    assert_eq!(p.env.get::<Node>(&recovering).liveliness_score, 1_225);
    for signer in signed.signer_pdas.iter().skip(1) {
        assert_eq!(
            p.env.get::<Node>(signer).liveliness_score,
            LIVELINESS_SCORE_MAX
        );
    }
    for node in missed {
        assert_eq!(
            p.env.get::<Node>(&node).liveliness_score,
            LIVELINESS_SCORE_MAX - 750
        );
    }
}

#[test]
fn rejected_attestation_does_not_change_liveliness() {
    let mut p = Protocol::builder().liveliness_penalty_per_miss(750).start();
    p.register_nodes("liveliness-invalid", NODE_COUNT);
    let gateway = p.register_gateway(500_000_000);
    let duty = Duty::new(NODE_COUNT as u32, 4);
    let selected = p.selection_bits(&duty);
    let fabricated = p
        .sign_duty_with_bits(duty, &selected[..4], None)
        .with_tampered_signature();

    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &fabricated.attestation(),
        p.x402_args(&fabricated, [0xbb; 32]),
    );
    let failed = p.env.send_err(&[ix], &[&gateway]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    for bit in selected {
        let node = p.env.registry(duty.registry_version).nodes[bit];
        assert_eq!(
            p.env.get::<Node>(&node).liveliness_score,
            LIVELINESS_SCORE_MAX
        );
    }
}
