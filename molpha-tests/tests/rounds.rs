//! Immediate round verification, reward accounting, and claiming.

use molpha::error::MolphaError;
use molpha::events::RewardsClaimed;
use molpha::state::Node;
use molpha_tests::env::{assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const FLOAT: u64 = 500_000_000;

#[test]
fn settlement_immediately_credits_signers_and_records_liability() {
    let mut p = Protocol::start();
    p.register_nodes("round-rewards", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    let signed = p.sign_duty(Duty::new(NODE_COUNT as u32, 4));
    let reward = p.args.reward_per_signature;

    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &signed.attestation(),
        p.x402_args(&signed, [0xaa; 32]),
    );
    let gateway = gateway.insecure_clone();
    p.env.send_ok(&[ix], &[&gateway]);

    for node in &signed.signer_pdas {
        assert_eq!(p.env.get::<Node>(node).claimable_rewards, reward);
    }
    assert_eq!(
        p.reward_liability(),
        reward * signed.signer_pdas.len() as u64
    );
    p.assert_solvent();
}

#[test]
fn invalid_attestation_is_rejected_before_payment_or_reward_changes() {
    let mut p = Protocol::start();
    p.register_nodes("invalid-round", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    let tampered = p
        .sign_duty(Duty::new(NODE_COUNT as u32, 4))
        .with_tampered_signature();
    let float = pda::ata(&gateway.pubkey(), &p.usdc_mint);
    let float_before = p.env.token_balance(&float);

    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &tampered.attestation(),
        p.x402_args(&tampered, [0xbb; 32]),
    );
    let failed = p.env.send_err(&[ix], &[&gateway]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    assert_eq!(p.env.token_balance(&float), float_before);
    assert_eq!(p.reward_liability(), 0);
    for node in &tampered.signer_pdas {
        assert_eq!(p.env.get::<Node>(node).claimable_rewards, 0);
    }
}

#[test]
fn a_node_owner_claims_rewards_credited_during_settlement() {
    let mut p = Protocol::start();
    p.register_nodes("claim-round", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    let signed = p.sign_duty(Duty::new(NODE_COUNT as u32, 4));
    let ix = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &signed.attestation(),
        p.x402_args(&signed, [0xcc; 32]),
    );
    p.env.send_ok(&[ix], &[&gateway]);

    let node_pda = signed.signer_pdas[0];
    let owner = p
        .nodes
        .iter()
        .find(|node| node.pda == node_pda)
        .expect("signer belongs to fixture")
        .owner
        .insecure_clone();
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance_or_zero(&destination);
    let reward = p.args.reward_per_signature;

    let ix = p.claim_node_rewards_ix(&owner.pubkey());
    let meta = p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(p.env.token_balance(&destination), before + reward);
    assert_eq!(p.env.get::<Node>(&node_pda).claimable_rewards, 0);
    assert_eq!(
        p.reward_liability(),
        reward * (signed.signer_pdas.len() as u64 - 1)
    );

    let event: RewardsClaimed = only_event(&meta);
    assert_eq!(event.beneficiary, owner.pubkey());
    assert_eq!(event.amount, reward);
}

#[test]
fn claiming_without_rewards_is_rejected() {
    let mut p = Protocol::start();
    p.register_nodes("empty-claim", NODE_COUNT);
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.claim_node_rewards_ix(&owner.pubkey());
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToClaim);
}
