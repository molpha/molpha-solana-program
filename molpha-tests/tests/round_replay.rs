//! Settlement replay markers: PDA scope, stored fields, and `close_round_replay` rent reclaim once
//! a round can never settle again.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::RoundReplayClosed;
use molpha::instructions::SettleX402RoundArgs;
use molpha::state::{
    PlanConfig, PlanType, RoundReplayKind, RoundReplayMarker, MAX_SETTLEMENT_DELAY_CAP_SECONDS,
};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use molpha_verifier::{Attestation, AttestationPayload, SchnorrSignature};
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
    p.register_nodes("round-replay", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    Fixture { p, gateway }
}

impl Fixture {
    fn gateway_pda(&self) -> Pubkey {
        pda::gateway(&self.gateway.pubkey())
    }

    fn settle_x402(&mut self, signed: &SignedAttestation, payer_id: [u8; 32]) -> Pubkey {
        let attestation = signed.attestation();
        let args = self.p.x402_args(signed, payer_id);
        let marker = self.marker(&attestation, &args);
        let ix = self
            .p
            .settle_x402_round_ix(&self.gateway.pubkey(), &attestation, args);
        let gateway = self.gateway.insecure_clone();
        self.p.env.send_ok(&[ix], &[&gateway]);
        marker
    }

    fn marker(
        &self,
        attestation: &molpha_verifier::payload::Attestation,
        args: &SettleX402RoundArgs,
    ) -> Pubkey {
        pda::x402_round(
            &args.payer_id,
            &self.gateway_pda(),
            &attestation.payload.source_id,
            attestation.payload.signatures_required,
            attestation.payload.canonical_timestamp,
        )
    }

    fn close_ix(
        &self,
        rent_receiver: &Pubkey,
        marker: &Pubkey,
    ) -> anchor_lang::solana_program::instruction::Instruction {
        self.p
            .close_round_replay_ix_for(&self.gateway_pda(), rent_receiver, marker)
    }

    /// Move cluster time past the point where no configuration can re-admit the round.
    fn expire(&mut self) {
        self.p
            .env
            .advance_seconds(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64 + 1);
    }
}

/// Golden vector for `RoundReplayMarker::round_id`, shared with `cli/tests/round-replay.cjs`.
#[test]
fn round_id_matches_the_shared_vector() {
    let attestation = Attestation {
        payload: AttestationPayload {
            value: [0; 32],
            source_id: [0x11; 32],
            registry_version: 0,
            signatures_required: 5,
            canonical_timestamp: 1_700_000_000,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0; 32],
            commitment: [0; 20],
            signers_bitmap: [0; 32],
        },
    };
    let id = RoundReplayMarker::round_id(RoundReplayKind::X402, &attestation, &[0x22; 32]);
    assert_eq!(hex(&id), ROUND_ID_VECTOR);
    // The kind tag separates the two settlement paths for identical inputs.
    let sub = RoundReplayMarker::round_id(RoundReplayKind::Subscription, &attestation, &[0x22; 32]);
    assert_ne!(sub, id);
}

const ROUND_ID_VECTOR: &str = "ccdd6789c23b7294903c844f1dc2645fee57072e356dfc4c2c9f575cd8ee0db6";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn settlement_records_the_round_id_and_canonical_timestamp() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let payer_id = [0x10; 32];
    let marker = f.settle_x402(&signed, payer_id);

    let stored: RoundReplayMarker = f.p.env.get(&marker);
    let expected_id =
        RoundReplayMarker::round_id(RoundReplayKind::X402, &signed.attestation(), &payer_id);
    assert_eq!(stored.round_id, expected_id);
    assert_eq!(stored.timestamp, signed.duty.canonical_timestamp);
    assert_eq!(marker, pda::round_replay(&f.gateway_pda(), &expected_id));
}

#[test]
fn an_expired_marker_closes_permissionlessly_and_refunds_the_gateway_authority() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let payer_id = [0x11; 32];
    let marker = f.settle_x402(&signed, payer_id);
    let stored: RoundReplayMarker = f.p.env.get(&marker);
    let rent = f.p.env.lamports(&marker);
    assert!(rent > 0);

    f.expire();
    let authority = f.gateway.pubkey();
    let before = f.p.env.lamports(&authority);
    // Any fee payer may crank the close; the rent still goes to the gateway authority.
    let cranker = f.p.env.keypair();
    let ix = f.close_ix(&authority, &marker);
    let meta = f.p.env.send_ok(&[ix], &[&cranker]);

    assert!(!f.p.env.account_exists(&marker));
    assert_eq!(f.p.env.lamports(&authority), before + rent);
    let closed: RoundReplayClosed = only_event(&meta);
    assert_eq!(closed.round_replay, marker);
    assert_eq!(closed.round_id, stored.round_id);
    assert_eq!(closed.gateway, f.gateway_pda());
    assert_eq!(closed.rent_receiver, authority);
    assert_eq!(closed.canonical_timestamp, signed.duty.canonical_timestamp);
}

#[test]
fn a_marker_cannot_close_while_the_capped_window_is_open() {
    let mut f = fixture();
    // The suite clock starts ahead of `Duty::new`'s hardcoded timestamp, so pin the round to now.
    let now = f.p.env.unix_timestamp() as u64;
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS).at(now));
    let marker = f.settle_x402(&signed, [0x22; 32]);

    // Exactly at the cap boundary the round is still (theoretically) admissible.
    f.p.env
        .advance_seconds(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64);
    let authority = f.gateway.pubkey();
    let cranker = f.p.env.keypair();
    let ix = f.close_ix(&authority, &marker);
    let failed = f.p.env.send_err(&[ix], &[&cranker]);
    assert_molpha_error(&failed, MolphaError::RoundReplayNotExpired);
    assert!(f.p.env.account_exists(&marker));
}

#[test]
fn a_closed_round_can_never_be_settled_again() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let payer_id = [0x33; 32];
    let marker = f.settle_x402(&signed, payer_id);
    f.expire();

    let authority = f.gateway.pubkey();
    let cranker = f.p.env.keypair();
    let ix = f.close_ix(&authority, &marker);
    f.p.env.send_ok(&[ix], &[&cranker]);

    // Even the widest configurable window no longer admits the round's timestamp.
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, payer_id);
    let ix = f.p.settle_x402_round_ix(&authority, &attestation, args);
    let gateway = f.gateway.insecure_clone();
    let failed = f.p.env.send_err(&[ix], &[&gateway]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);
}

#[test]
fn rent_can_only_go_to_the_gateway_authority() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let marker = f.settle_x402(&signed, [0x44; 32]);
    f.expire();

    let thief = f.p.env.keypair();
    let ix = f.close_ix(&thief.pubkey(), &marker);
    let failed = f.p.env.send_err(&[ix], &[&thief]);
    assert_molpha_error(&failed, MolphaError::InvalidRentReceiver);
    assert!(f.p.env.account_exists(&marker));
}

/// Without the seeds binding, another gateway's authority could collect the marker's rent.
#[test]
fn a_marker_cannot_be_closed_against_another_gateway() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let marker = f.settle_x402(&signed, [0x45; 32]);
    f.expire();

    let other = f.p.register_gateway(0);
    let ix =
        f.p.close_round_replay_ix_for(&pda::gateway(&other.pubkey()), &other.pubkey(), &marker);
    let failed = f.p.env.send_err(&[ix], &[&other]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
    assert!(f.p.env.account_exists(&marker));
}

/// The kind tag keeps the two paths apart: an x402 `payer_id` equal to a subscriber's key bytes
/// does not block that subscriber's settlement of the same round through the same gateway.
#[test]
fn subscription_and_x402_markers_never_collide() {
    let mut p = Protocol::start();
    p.register_nodes("round-replay-kinds", NODE_COUNT);
    p.add_plan(PlanConfig {
        plan_type: PlanType::Professional,
        subscription_price: 10_000_000,
        max_signers: 6,
        max_delegates: 1,
        max_rounds: 3,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = p.subscribe(PlanType::Professional);
    let gateway = p.register_gateway(FLOAT);
    let gateway_signer = gateway.insecure_clone();
    let signed = p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));

    let payer_id = owner.pubkey().to_bytes();
    let args = p.x402_args(&signed, payer_id);
    let ix = p.settle_x402_round_ix(&gateway.pubkey(), &signed.attestation(), args);
    p.env.send_ok(&[ix], &[&gateway_signer]);

    let ixs = vec![
        p.consumer_authorization_ix(&owner, &gateway.pubkey(), &signed.duty),
        p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    p.env.send_ok(&ixs, &[&gateway_signer]);

    let gateway_pda = pda::gateway(&gateway.pubkey());
    let (source_id, sigs, ts) = (
        signed.duty.source_id,
        signed.duty.signatures_required,
        signed.duty.canonical_timestamp,
    );
    let x402 = pda::x402_round(&payer_id, &gateway_pda, &source_id, sigs, ts);
    let sub = pda::subscription_round(&owner.pubkey(), &gateway_pda, &source_id, sigs, ts);
    assert_ne!(x402, sub);
    assert!(p.env.account_exists(&x402));
    assert!(p.env.account_exists(&sub));
}

#[test]
fn subscription_markers_close_after_the_window() {
    let mut p = Protocol::start();
    p.register_nodes("round-replay-sub", NODE_COUNT);
    p.add_plan(PlanConfig {
        plan_type: PlanType::Professional,
        subscription_price: 10_000_000,
        max_signers: 6,
        max_delegates: 1,
        max_rounds: 3,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = p.subscribe(PlanType::Professional);
    let gateway = p.register_gateway(0);
    let signed = p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));

    let ixs = vec![
        p.consumer_authorization_ix(&owner, &gateway.pubkey(), &signed.duty),
        p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    let gateway_signer = gateway.insecure_clone();
    p.env.send_ok(&ixs, &[&gateway_signer]);
    let marker = pda::subscription_round(
        &owner.pubkey(),
        &pda::gateway(&gateway.pubkey()),
        &signed.duty.source_id,
        signed.duty.signatures_required,
        signed.duty.canonical_timestamp,
    );
    assert!(p.env.account_exists(&marker));

    p.env
        .advance_seconds(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64 + 1);
    let cranker = p.env.keypair();
    let ix =
        p.close_round_replay_ix_for(&pda::gateway(&gateway.pubkey()), &gateway.pubkey(), &marker);
    p.env.send_ok(&[ix], &[&cranker]);
    assert!(!p.env.account_exists(&marker));
}
