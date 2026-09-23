//! `close_round_replay`: rent reclaim for settlement replay markers once a round can never
//! settle again, and the settlement-delay cap that makes the close safe.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::RoundReplayClosed;
use molpha::instructions::SettleX402RoundArgs;
use molpha::state::{PlanConfig, PlanType, MAX_SETTLEMENT_DELAY_CAP_SECONDS};
use molpha_tests::env::{assert_molpha_error, only_event};
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
    p.register_nodes("round-replay", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    Fixture { p, gateway }
}

impl Fixture {
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
            &pda::gateway(&self.gateway.pubkey()),
            &attestation.payload.source_id,
            attestation.payload.signatures_required,
            attestation.payload.canonical_timestamp,
        )
    }

    /// Move cluster time past the point where no configuration can re-admit the round.
    fn expire(&mut self) {
        self.p
            .env
            .advance_seconds(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64 + 1);
    }
}

#[test]
fn an_expired_marker_closes_permissionlessly_and_refunds_the_gateway_authority() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let payer_id = [0x11; 32];
    let marker = f.settle_x402(&signed, payer_id);
    let rent = f.p.env.lamports(&marker);
    assert!(rent > 0);

    f.expire();
    let authority = f.gateway.pubkey();
    let before = f.p.env.lamports(&authority);
    // Any fee payer may crank the close; the rent still goes to the gateway authority.
    let cranker = f.p.env.keypair();
    let ix =
        f.p.close_round_replay_ix_for(&pda::gateway(&authority), &authority, &marker);
    let meta = f.p.env.send_ok(&[ix], &[&cranker]);

    assert!(!f.p.env.account_exists(&marker));
    assert_eq!(f.p.env.lamports(&authority), before + rent);
    let closed: RoundReplayClosed = only_event(&meta);
    assert_eq!(closed.round_replay, marker);
    assert_eq!(closed.rent_receiver, authority);
}

#[test]
fn a_marker_cannot_close_while_the_capped_window_is_open() {
    let mut f = fixture();
    let signed = f.p.sign_duty(Duty::new(NODE_COUNT as u32, SIGS));
    let payer_id = [0x22; 32];
    let marker = f.settle_x402(&signed, payer_id);

    // Exactly at the cap boundary the round is still (theoretically) admissible.
    f.p.env
        .advance_seconds(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64);
    let authority = f.gateway.pubkey();
    let cranker = f.p.env.keypair();
    let ix =
        f.p.close_round_replay_ix_for(&pda::gateway(&authority), &authority, &marker);
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
    let ix =
        f.p.close_round_replay_ix_for(&pda::gateway(&authority), &authority, &marker);
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
    let payer_id = [0x44; 32];
    let marker = f.settle_x402(&signed, payer_id);
    f.expire();

    let thief = f.p.env.keypair();
    let ix =
        f.p.close_round_replay_ix_for(&pda::gateway(&f.gateway.pubkey()), &thief.pubkey(), &marker);
    let failed = f.p.env.send_err(&[ix], &[&thief]);
    assert_molpha_error(&failed, MolphaError::InvalidRentReceiver);
    assert!(f.p.env.account_exists(&marker));
}

#[test]
fn subscription_markers_close_under_their_own_prefix() {
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
