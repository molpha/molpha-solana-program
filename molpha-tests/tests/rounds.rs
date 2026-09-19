//! `finalize_round`, `validate_round` and `claim_node_rewards` — the tail of the round lifecycle.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{GatewaySlashed, RewardsClaimed, RoundFinalized, RoundVoided};
use molpha::state::{
    Gateway, GatewayStatus, Node, NodeStatus, PlanConfig, PlanType, RoundReceiptRecord,
    RoundRecordStatus, Subscription,
};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 4;
const FLOAT: u64 = 500_000_000;

struct Fixture {
    p: Protocol,
    gateway: Keypair,
}

fn fixture() -> Fixture {
    let mut p = Protocol::start();
    p.register_nodes("rounds", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    Fixture { p, gateway }
}

fn duty() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS)
}

impl Fixture {
    fn gateway_pda(&self) -> Pubkey {
        pda::gateway(&self.gateway.pubkey())
    }

    fn bond_ata(&self) -> Pubkey {
        pda::ata(&self.gateway_pda(), &self.p.usdc_mint)
    }

    /// Settle an x402 round. `signed` may carry a signature that does not verify — settlement is
    /// optimistic and only checks the round's *shape*.
    fn settle_x402(&mut self, signed: &SignedAttestation, payer_id: [u8; 32]) -> Pubkey {
        let attestation = signed.attestation();
        let args = self.p.x402_args(signed, payer_id);
        let record = pda::x402_round(
            &args.payer_id,
            &self.gateway_pda(),
            &attestation.payload.source_id,
            attestation.payload.signatures_required,
            attestation.payload.canonical_timestamp,
        );
        let ix = self
            .p
            .settle_x402_round_ix(&self.gateway.pubkey(), &attestation, args);
        let gateway = self.gateway.insecure_clone();
        self.p.env.send_ok(&[ix], &[&gateway]);
        record
    }

    /// A subscriber with a plan generous enough for the tests below.
    fn subscribe(&mut self) -> Keypair {
        self.p.add_plan(PlanConfig {
            plan_type: PlanType::Professional,
            subscription_price: 10_000_000,
            max_signers: 8,
            max_delegates: 2,
            max_rounds: 10,
            private_api_enabled: true,
            is_active: true,
        });
        self.p.subscribe(PlanType::Professional)
    }

    fn settle_subscription(&mut self, owner: &Keypair, signed: &SignedAttestation) -> Pubkey {
        let ixs = vec![
            self.p
                .consumer_authorization_ix(owner, &self.gateway.pubkey(), &signed.duty),
            self.p.settle_subscription_round_ix(
                &self.gateway.pubkey(),
                &owner.pubkey(),
                &owner.pubkey(),
                None,
                signed,
            ),
        ];
        let gateway = self.gateway.insecure_clone();
        let owner_kp = owner.insecure_clone();
        self.p.env.send_ok(&ixs, &[&owner_kp, &gateway]);
        pda::subscription_round(
            &owner.pubkey(),
            &self.gateway_pda(),
            &signed.duty.source_id,
            signed.duty.signatures_required,
            signed.duty.canonical_timestamp,
        )
    }

    fn credited_nodes(&self, record: &Pubkey) -> Vec<Pubkey> {
        let record: RoundReceiptRecord = self.p.env.get(record);
        self.p
            .node_pdas_for_bitmap(record.registry_version, &record.signers_bitmap)
    }

    fn past_the_dispute_window(&mut self) {
        let window = self.p.args.dispute_window_slots;
        self.p.env.advance_slots(window + 1);
    }
}

// ---------------------------------------------------------------------------
// finalize_round
// ---------------------------------------------------------------------------

#[test]
fn finalize_credits_every_signer_and_closes_the_round() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    assert_eq!(
        credited.len(),
        SIGS as usize + f.p.args.redundancy_buffer as usize
    );

    let before: Vec<u64> = credited
        .iter()
        .map(|n| f.p.env.get::<Node>(n).claimable_rewards)
        .collect();

    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let meta = f.p.env.send_ok(&[ix], &[&caller]);

    let reward = f.p.args.reward_per_signature;
    for (node, previous) in credited.iter().zip(before) {
        assert_eq!(
            f.p.env.get::<Node>(node).claimable_rewards,
            previous + reward
        );
    }
    // Nodes outside the signer set get nothing.
    for node in f.p.node_pdas() {
        if !credited.contains(&node) {
            assert_eq!(f.p.env.get::<Node>(&node).claimable_rewards, 0);
        }
    }

    let updated: RoundReceiptRecord = f.p.env.get(&record);
    assert_eq!(updated.status, RoundRecordStatus::Finalized);

    let event: RoundFinalized = only_event(&meta);
    assert_eq!(event.round_record, record);
}

#[test]
fn finalize_is_permissionless() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();

    // A stranger with no relationship to the round may finalize it.
    let stranger = f.p.env.keypair();
    let ix =
        f.p.finalize_round_ix(&stranger.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&stranger]);
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
}

#[test]
fn finalize_waits_for_the_dispute_window_to_close() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    let caller = f.p.env.keypair();

    // Immediately: too early.
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::RoundNotFinalizable);

    // Exactly `dispute_window_slots` later: still too early, the guard is strict.
    let window = f.p.args.dispute_window_slots;
    f.p.env.advance_slots(window);
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::RoundNotFinalizable);

    // One slot past the window: allowed.
    f.p.env.advance_slots(1);
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);
}

#[test]
fn a_round_can_only_be_finalized_once() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();

    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let rewards_after_first: Vec<u64> = credited
        .iter()
        .map(|n| f.p.env.get::<Node>(n).claimable_rewards)
        .collect();

    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::RoundNotPending);

    for (node, expected) in credited.iter().zip(rewards_after_first) {
        assert_eq!(f.p.env.get::<Node>(node).claimable_rewards, expected);
    }
}

#[test]
fn finalize_rejects_a_credited_account_list_that_does_not_match_the_bitmap() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();

    // Too few accounts.
    let ix =
        f.p.finalize_round_ix(&caller.pubkey(), &record, &credited[..credited.len() - 1]);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidMissedNodeAccount);

    // Right count, wrong order.
    let mut shuffled = credited.clone();
    shuffled.swap(0, 1);
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &shuffled);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidCreditNodeAccount);

    // Right count, an uncredited node substituted in.
    let uncredited =
        f.p.node_pdas()
            .into_iter()
            .find(|n| !credited.contains(n))
            .expect("some node did not sign");
    let mut substituted = credited.clone();
    substituted[0] = uncredited;
    let ix =
        f.p.finalize_round_ix(&caller.pubkey(), &record, &substituted);
    let failed = f.p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidCreditNodeAccount);
}

#[test]
fn finalize_credits_a_node_that_has_since_been_deactivated() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);

    // A signer leaves the registry after settling but before finalization.
    let leaving = credited[0];
    let index =
        f.p.nodes
            .iter()
            .position(|n| n.pda == leaving)
            .expect("credited node is one of ours");
    let owner = f.p.nodes[index].owner.insecure_clone();
    let ix = f.p.deactivate_node_ix(index, &owner.pubkey());
    f.p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(
        f.p.env.get::<Node>(&leaving).status,
        NodeStatus::Deactivated
    );

    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    // The round is pinned to its own registry snapshot, so work already done still pays.
    assert_eq!(
        f.p.env.get::<Node>(&leaving).claimable_rewards,
        f.p.args.reward_per_signature
    );
}

// ---------------------------------------------------------------------------
// validate_round
// ---------------------------------------------------------------------------

#[test]
fn validating_a_fabricated_x402_round_voids_it_and_slashes_the_bond() {
    let mut f = fixture();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);

    let challenger = f.p.env.keypair();
    let bond = f.p.env.token_balance(&f.bond_ata());
    let treasury_before = f.p.treasury_balance();
    let expected_bounty = (bond as u128 * f.p.args.challenger_bounty_bps as u128 / 10_000) as u64;
    assert!(expected_bounty > 0);

    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let meta = f.p.env.send_ok(&[ix], &[&challenger]);

    let updated: RoundReceiptRecord = f.p.env.get(&record);
    assert_eq!(updated.status, RoundRecordStatus::Voided);

    let gateway: Gateway = f.p.env.get(&f.gateway_pda());
    assert_eq!(gateway.status, GatewayStatus::Slashed);
    assert_eq!(f.p.env.token_balance(&f.bond_ata()), 0);

    let challenger_usdc = pda::ata(&challenger.pubkey(), &f.p.usdc_mint);
    assert_eq!(f.p.env.token_balance(&challenger_usdc), expected_bounty);
    assert_eq!(
        f.p.treasury_balance(),
        treasury_before + bond - expected_bounty
    );

    let voided_event: RoundVoided = only_event(&meta);
    assert_eq!(voided_event.round_record, record);
    assert!(
        !voided_event.round_restored,
        "x402 rounds are paid up front and are not refunded"
    );

    let gateway_slashed_event: GatewaySlashed = only_event(&meta);
    assert_eq!(gateway_slashed_event.gateway, f.gateway_pda());
    assert_eq!(gateway_slashed_event.slashed_bond, bond);
    assert_eq!(gateway_slashed_event.challenger_bounty, expected_bounty);
}

#[test]
fn validating_a_fabricated_subscription_round_gives_the_round_back() {
    let mut f = fixture();
    let owner = f.subscribe();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_subscription(&owner, &fabricated);
    assert_eq!(
        f.p.env
            .get::<Subscription>(&pda::subscription(&owner.pubkey()))
            .used_rounds,
        1
    );

    let signers = f.credited_nodes(&record);
    let challenger = f.p.env.keypair();
    let ix = f.p.validate_round_ix(
        &challenger.pubkey(),
        &record,
        Some(pda::subscription(&owner.pubkey())),
        &signers,
    );
    let meta = f.p.env.send_ok(&[ix], &[&challenger]);

    assert_eq!(
        f.p.env
            .get::<Subscription>(&pda::subscription(&owner.pubkey()))
            .used_rounds,
        0,
        "a voided round is refunded to the subscription"
    );
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Voided
    );
    let event: RoundVoided = only_event(&meta);
    assert!(event.round_restored);
}

#[test]
fn validating_a_genuine_round_finalizes_it_and_credits_its_signers() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let signers = f.credited_nodes(&record);
    let rewards_before: Vec<u64> = signers
        .iter()
        .map(|node| f.p.env.get::<Node>(node).claimable_rewards)
        .collect();

    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let meta = f.p.env.send_ok(&[ix], &[&challenger]);

    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    for (node, before) in signers.iter().zip(rewards_before) {
        assert_eq!(
            f.p.env.get::<Node>(node).claimable_rewards,
            before + f.p.args.reward_per_signature
        );
    }

    let event: RoundFinalized = only_event(&meta);
    assert_eq!(event.round_record, record);

    let rewards_after: Vec<u64> = signers
        .iter()
        .map(|node| f.p.env.get::<Node>(node).claimable_rewards)
        .collect();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::RoundNotPending);
    for (node, after) in signers.iter().zip(rewards_after) {
        assert_eq!(f.p.env.get::<Node>(node).claimable_rewards, after);
    }

    let gateway: Gateway = f.p.env.get(&f.gateway_pda());
    assert_eq!(gateway.status, GatewayStatus::Active);
    assert_eq!(
        f.p.env.token_balance(&f.bond_ata()),
        f.p.args.gateway_bond_min
    );
}

#[test]
fn a_deactivated_gateway_remains_slashable_through_the_dispute_boundary() {
    let mut f = fixture();
    f.p.env.advance_seconds(1);
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xc1; 32]);
    let signers = f.credited_nodes(&record);
    let bond = f.p.env.token_balance(&f.bond_ata());

    let gateway = f.gateway.insecure_clone();
    let protocol_authority = f.p.authority.insecure_clone();
    let deactivate =
        f.p.deactivate_gateway_ix(&protocol_authority.pubkey(), &gateway.pubkey());
    f.p.env.send_ok(&[deactivate], &[&protocol_authority]);
    assert_eq!(
        f.p.env.get::<Gateway>(&f.gateway_pda()).status,
        GatewayStatus::Deactivated
    );
    assert_eq!(f.p.env.token_balance(&f.bond_ata()), bond);

    f.p.env.advance_slots(f.p.args.dispute_window_slots);
    let challenger = f.p.env.keypair();
    let validation =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[validation], &[&challenger]);

    let gateway_state: Gateway = f.p.env.get(&f.gateway_pda());
    assert_eq!(gateway_state.status, GatewayStatus::Slashed);
    assert_eq!(f.p.env.token_balance(&f.bond_ata()), 0);

    f.p.env.advance_slots(1);
    let withdraw = f.p.withdraw_gateway_bond_ix(&gateway.pubkey());
    let failed = f.p.env.send_err(&[withdraw], &[&gateway]);
    assert_molpha_error(&failed, MolphaError::GatewayNotDeactivated);
}

#[test]
fn a_fabricated_round_can_still_be_disputed_after_its_window() {
    let mut f = fixture();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xc2; 32]);
    let signers = f.credited_nodes(&record);
    f.past_the_dispute_window();

    let challenger = f.p.env.keypair();
    let validation =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[validation], &[&challenger]);

    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Voided
    );
    assert_eq!(
        f.p.env.get::<Gateway>(&f.gateway_pda()).status,
        GatewayStatus::Slashed
    );
}

#[test]
fn a_genuine_round_can_still_be_validated_after_its_window() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xc3; 32]);
    let signers = f.credited_nodes(&record);
    let rewards_before: Vec<u64> = signers
        .iter()
        .map(|node| f.p.env.get::<Node>(node).claimable_rewards)
        .collect();
    f.past_the_dispute_window();

    let challenger = f.p.env.keypair();
    let validation =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[validation], &[&challenger]);

    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    for (node, before) in signers.iter().zip(rewards_before) {
        assert_eq!(
            f.p.env.get::<Node>(node).claimable_rewards,
            before + f.p.args.reward_per_signature
        );
    }
}

#[test]
fn an_x402_validation_must_not_name_a_subscription() {
    let mut f = fixture();
    let owner = f.subscribe();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);

    let challenger = f.p.env.keypair();
    let ix = f.p.validate_round_ix(
        &challenger.pubkey(),
        &record,
        Some(pda::subscription(&owner.pubkey())),
        &signers,
    );
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::WrongSubscriptionAccount);
}

#[test]
fn a_subscription_validation_must_name_the_subscription() {
    let mut f = fixture();
    let owner = f.subscribe();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_subscription(&owner, &fabricated);
    let signers = f.credited_nodes(&record);

    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::WrongSubscriptionAccount);
}

#[test]
fn a_round_can_only_be_validated_once() {
    let mut f = fixture();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);
    let challenger = f.p.env.keypair();

    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[ix], &[&challenger]);

    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::RoundNotPending);
}

#[test]
fn a_voided_round_can_never_be_finalized() {
    let mut f = fixture();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);
    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[ix], &[&challenger]);

    f.past_the_dispute_window();
    let ix =
        f.p.finalize_round_ix(&challenger.pubkey(), &record, &signers);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::RoundNotPending);

    for node in signers {
        assert_eq!(f.p.env.get::<Node>(&node).claimable_rewards, 0);
    }
}

#[test]
fn validation_must_name_the_gateway_that_settled_the_round() {
    let mut f = fixture();
    let other_gateway = f.p.register_gateway(FLOAT);
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);

    let challenger = f.p.env.keypair();
    let mut ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    let settled_by = f.gateway_pda();
    let innocent = pda::gateway(&other_gateway.pubkey());
    for meta in ix.accounts.iter_mut() {
        if meta.pubkey == settled_by {
            meta.pubkey = innocent;
        } else if meta.pubkey == pda::ata(&settled_by, &f.p.usdc_mint) {
            meta.pubkey = pda::ata(&innocent, &f.p.usdc_mint);
        }
    }
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::GatewayIndexMismatch);
}

#[test]
fn a_slashed_gateway_can_no_longer_settle() {
    let mut f = fixture();
    let fabricated = f.p.sign_duty(duty()).with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0xaa; 32]);
    let signers = f.credited_nodes(&record);
    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signers);
    f.p.env.send_ok(&[ix], &[&challenger]);

    let signed = f.p.sign_duty(duty().at(1_700_000_999));
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xbb; 32]);
    let ix =
        f.p.settle_x402_round_ix(&f.gateway.pubkey(), &attestation, args);
    let gateway = f.gateway.insecure_clone();
    let failed = f.p.env.send_err(&[ix], &[&gateway]);
    assert_molpha_error(&failed, MolphaError::GatewayNotActive);
}

#[test]
fn a_slashed_gateways_valid_rounds_must_finalize_through_validation() {
    let mut f = fixture();
    let challenger = f.p.env.keypair();

    let valid_attestation = f.p.sign_duty(duty());
    let valid_record = f.settle_x402(&valid_attestation, [0xa1; 32]);
    let valid_signers = f.credited_nodes(&valid_record);

    let pending_attestation = f.p.sign_duty(duty().at(1_700_000_001));
    let pending_record = f.settle_x402(&pending_attestation, [0xa2; 32]);
    let pending_signers = f.credited_nodes(&pending_record);

    let fabricated =
        f.p.sign_duty(duty().at(1_700_000_002))
            .with_tampered_signature();
    let fabricated_record = f.settle_x402(&fabricated, [0xa3; 32]);
    let fabricated_signers = f.credited_nodes(&fabricated_record);
    let ix = f.p.validate_round_ix(
        &challenger.pubkey(),
        &fabricated_record,
        None,
        &fabricated_signers,
    );
    f.p.env.send_ok(&[ix], &[&challenger]);
    assert_eq!(
        f.p.env.get::<Gateway>(&f.gateway_pda()).status,
        GatewayStatus::Slashed
    );

    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &valid_record, None, &valid_signers);
    f.p.env.send_ok(&[ix], &[&challenger]);
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&valid_record).status,
        RoundRecordStatus::Finalized
    );

    f.past_the_dispute_window();

    let ix =
        f.p.finalize_round_ix(&challenger.pubkey(), &pending_record, &pending_signers);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::GatewaySlashed);
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&pending_record).status,
        RoundRecordStatus::Pending
    );
}

// ---------------------------------------------------------------------------
// claim_node_rewards
// ---------------------------------------------------------------------------

#[test]
fn a_node_owner_withdraws_finalized_rewards_from_the_treasury() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let index =
        f.p.nodes
            .iter()
            .position(|n| n.pda == credited[0])
            .expect("credited node is ours");
    let owner = f.p.nodes[index].owner.insecure_clone();
    let destination = pda::ata(&owner.pubkey(), &f.p.usdc_mint);
    let owner_before = f.p.env.token_balance_or_zero(&destination);
    let treasury_before = f.p.treasury_balance();
    let reward = f.p.args.reward_per_signature;

    let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
    let meta = f.p.env.send_ok(&[ix], &[&owner]);

    assert_eq!(f.p.env.token_balance(&destination), owner_before + reward);
    assert_eq!(f.p.treasury_balance(), treasury_before - reward);
    assert_eq!(f.p.env.get::<Node>(&credited[0]).claimable_rewards, 0);

    let event: RewardsClaimed = only_event(&meta);
    assert_eq!(event.beneficiary, owner.pubkey());
    assert_eq!(event.amount, reward);
    f.p.assert_solvent();
}

#[test]
fn claiming_twice_is_refused_because_nothing_is_left() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let index = f.p.nodes.iter().position(|n| n.pda == credited[0]).unwrap();
    let owner = f.p.nodes[index].owner.insecure_clone();
    let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
    f.p.env.send_ok(&[ix], &[&owner]);

    let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
    let failed = f.p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToClaim);
}

#[test]
fn a_node_with_no_finalized_work_has_nothing_to_claim() {
    let mut f = fixture();
    let owner = f.p.nodes[0].owner.insecure_clone();
    let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
    let failed = f.p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToClaim);
}

#[test]
fn only_the_node_owner_can_claim_its_rewards() {
    let mut f = fixture();
    let stranger = f.p.env.keypair();
    // The node PDA is owner-seeded, so a stranger's claim names an account that does not exist.
    let ix = f.p.claim_node_rewards_ix(&stranger.pubkey());
    let failed = f.p.env.send_err(&[ix], &[&stranger]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
}

#[test]
fn a_deactivated_node_can_still_claim_rewards_it_already_earned() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let index = f.p.nodes.iter().position(|n| n.pda == credited[0]).unwrap();
    let owner = f.p.nodes[index].owner.insecure_clone();
    let ix = f.p.deactivate_node_ix(index, &owner.pubkey());
    f.p.env.send_ok(&[ix], &[&owner]);

    let destination = pda::ata(&owner.pubkey(), &f.p.usdc_mint);
    let before = f.p.env.token_balance_or_zero(&destination);
    let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
    f.p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(
        f.p.env.token_balance(&destination),
        before + f.p.args.reward_per_signature
    );
}

// ---------------------------------------------------------------------------
// treasury solvency
// ---------------------------------------------------------------------------

/// Rewards a subscription plan does not pay for.
///
/// Subscription revenue is a flat monthly price, so a plan can promise more rounds than it funds.
/// Crediting refuses rather than quietly paying node principal, which the treasury no longer
/// holds anyway.
const RICH_REWARD: u64 = 10_000_000;

fn underfunded_fixture() -> (Fixture, Keypair) {
    let mut p = Protocol::builder()
        .reward_per_signature(RICH_REWARD)
        .start();
    p.register_nodes("insolvent", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    let mut f = Fixture { p, gateway };
    f.p.add_plan(PlanConfig {
        plan_type: PlanType::Professional,
        subscription_price: RICH_REWARD,
        max_signers: 8,
        max_delegates: 2,
        max_rounds: 10,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = f.p.subscribe(PlanType::Professional);
    (f, owner)
}

#[test]
fn finalize_is_refused_while_the_treasury_cannot_cover_the_credit() {
    let (mut f, owner) = underfunded_fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_subscription(&owner, &signed);
    let credited = f.credited_nodes(&record);
    let owed = RICH_REWARD * credited.len() as u64;
    assert!(f.p.treasury_balance() < owed, "the month's price is short");

    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let failed = f.p.env.send_err(std::slice::from_ref(&ix), &[&caller]);
    assert_molpha_error(&failed, MolphaError::TreasuryInsolvent);

    // Nothing moved: the round is still finalizable later.
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Pending
    );
    assert_eq!(f.p.reward_liability(), 0);
    for node in &credited {
        assert_eq!(f.p.env.get::<Node>(node).claimable_rewards, 0);
    }

    // A plain SPL transfer into the treasury is all it takes to make the round finalizable.
    let short = owed - f.p.treasury_balance();
    f.p.fund_treasury(short);
    f.p.env.send_ok(&[ix], &[&caller]);

    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
    assert_eq!(f.p.reward_liability(), owed);
    assert_eq!(f.p.treasury_balance(), owed);
    f.p.assert_solvent();
}

#[test]
fn validating_a_sound_signature_is_bound_by_the_same_solvency_check() {
    let (mut f, owner) = underfunded_fixture();
    let signed = f.p.sign_duty(duty());
    let record = f.settle_subscription(&owner, &signed);
    let credited = f.credited_nodes(&record);

    // The signature is genuine, so `validate_round` takes the crediting branch, not the slash one.
    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &credited);
    let failed = f.p.env.send_err(&[ix], &[&challenger]);
    assert_molpha_error(&failed, MolphaError::TreasuryInsolvent);

    assert_eq!(
        f.p.env.get::<Gateway>(&f.gateway_pda()).status,
        GatewayStatus::Active
    );
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Pending
    );
}

#[test]
fn an_x402_round_always_funds_the_rewards_it_credits() {
    let mut f = fixture();
    assert_eq!(f.p.treasury_balance(), 0, "no other revenue is in play");

    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();

    // The price charged the whole selection, so whatever subset signed is covered.
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let owed = f.p.args.reward_per_signature * credited.len() as u64;
    assert_eq!(f.p.reward_liability(), owed);
    assert!(
        f.p.treasury_balance() >= owed,
        "the round paid for its own rewards"
    );
}

#[test]
fn reward_liability_follows_every_credit_and_claim() {
    let mut f = fixture();
    assert_eq!(f.p.reward_liability(), 0);

    let signed = f.p.sign_duty(duty());
    let record = f.settle_x402(&signed, [0xaa; 32]);
    let credited = f.credited_nodes(&record);
    f.past_the_dispute_window();
    let caller = f.p.env.keypair();
    let ix = f.p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    f.p.env.send_ok(&[ix], &[&caller]);

    let reward = f.p.args.reward_per_signature;
    assert_eq!(f.p.reward_liability(), reward * credited.len() as u64);

    // Each claim retires exactly its own share of the liability.
    for (claimed, node) in credited.iter().enumerate() {
        let index = f.p.nodes.iter().position(|n| n.pda == *node).unwrap();
        let owner = f.p.nodes[index].owner.insecure_clone();
        let ix = f.p.claim_node_rewards_ix(&owner.pubkey());
        f.p.env.send_ok(&[ix], &[&owner]);
        assert_eq!(
            f.p.reward_liability(),
            reward * (credited.len() - claimed - 1) as u64
        );
    }
    assert_eq!(f.p.reward_liability(), 0);
    assert_eq!(
        f.p.env
            .get::<molpha::state::ProtocolConfig>(&pda::protocol_config())
            .reward_liability,
        0
    );
}
