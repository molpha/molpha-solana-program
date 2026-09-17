//! `settle_x402_round`: gateway-funded pay-per-request settlement.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::X402RoundSettled;
use molpha::instructions::SettleX402RoundArgs;
use molpha::state::{
    Gateway, GatewayStatus, RoundReceiptRecord, RoundRecordKind, RoundRecordStatus,
};
use molpha::utils::MAX_FUTURE_DRIFT_SECONDS;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

use molpha_verifier::payload::Attestation;

const NODE_COUNT: usize = 8;
const FLOAT: u64 = 500_000_000;

struct Fixture {
    p: Protocol,
    gateway: Keypair,
}

fn fixture() -> Fixture {
    let mut p = Protocol::start();
    p.register_nodes("settle-x402", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    Fixture { p, gateway }
}

impl Fixture {
    fn gateway_pda(&self) -> Pubkey {
        pda::gateway(&self.gateway.pubkey())
    }

    fn float_ata(&self) -> Pubkey {
        pda::ata(&self.gateway.pubkey(), &self.p.usdc_mint)
    }

    fn bond_ata(&self) -> Pubkey {
        pda::ata(&self.gateway_pda(), &self.p.usdc_mint)
    }

    fn record(&self, attestation: &Attestation, args: &SettleX402RoundArgs) -> Pubkey {
        pda::x402_round(
            &args.payer_id,
            &self.gateway_pda(),
            &attestation.payload.source_id,
            attestation.payload.signatures_required,
            attestation.payload.canonical_timestamp,
        )
    }

    fn settle(
        &mut self,
        attestation: &Attestation,
        args: SettleX402RoundArgs,
    ) -> litesvm::types::TransactionResult {
        let ix = self
            .p
            .settle_x402_round_ix(&self.gateway.pubkey(), &attestation, args);
        let gateway = self.gateway.insecure_clone();
        self.p.env.send(&[ix], &[&gateway])
    }
}

fn duty(signatures_required: u8) -> Duty {
    Duty::new(NODE_COUNT as u32, signatures_required)
}

fn x402_settlement(
    p: &Protocol,
    signed: &molpha_tests::scenario::SignedAttestation,
    payer_id: [u8; 32],
) -> (Attestation, SettleX402RoundArgs) {
    (signed.attestation(), p.x402_args(signed, payer_id))
}

/// The whole selection is charged for, not just the threshold: finalization may credit up to
/// `signatures_required + redundancy_buffer` signers.
fn expected_price(p: &Protocol, signatures_required: u8) -> u64 {
    p.args.x402_round_base
        + u64::from(signatures_required + p.args.redundancy_buffer) * p.args.reward_per_signature
}

#[test]
fn settlement_debits_the_gateway_float_and_credits_the_treasury() {
    let mut f = fixture();
    let duty = duty(4);
    let signed = f.p.sign_duty(duty);
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);
    let price = expected_price(&f.p, 4);
    assert_eq!(args.amount, price);

    let float_before = f.p.env.token_balance(&f.float_ata());
    let bond_before = f.p.env.token_balance(&f.bond_ata());
    let treasury_before = f.p.treasury_balance();

    let ix =
        f.p.settle_x402_round_ix(&f.gateway.pubkey(), &attestation, args.clone());
    let gateway = f.gateway.insecure_clone();
    let meta = f.p.env.send_ok(&[ix], &[&gateway]);

    assert_eq!(f.p.env.token_balance(&f.float_ata()), float_before - price);
    assert_eq!(f.p.treasury_balance(), treasury_before + price);
    assert_eq!(
        f.p.env.token_balance(&f.bond_ata()),
        bond_before,
        "the bond is not a payment source"
    );

    let record: RoundReceiptRecord = f.p.env.get(&f.record(&attestation, &args));
    assert_eq!(record.status, RoundRecordStatus::Pending);
    assert_eq!(record.kind, RoundRecordKind::X402);
    assert_eq!(record.gateway, f.gateway_pda());
    assert_eq!(record.registry_version, duty.registry_version);
    assert_eq!(record.signers_bitmap, signed.signers_bitmap);
    assert_eq!(record.attestation_message_hash, signed.message_hash);
    assert_eq!(record.reward_per_signature, f.p.args.reward_per_signature);

    let event: X402RoundSettled = only_event(&meta);
    assert_eq!(event.payer_id, args.payer_id);
    assert_eq!(event.payment_rail, args.payment_rail);
    assert_eq!(event.price, price);
    assert_eq!(event.gateway, f.gateway_pda());
}

#[test]
fn the_price_scales_with_the_requested_quorum() {
    let mut f = fixture();
    for signatures_required in [3u8, 5, 7] {
        let signed = f.p.sign_duty(duty(signatures_required));
        let attestation = signed.attestation();
        let args = f.p.x402_args(&signed, [signatures_required; 32]);
        assert_eq!(args.amount, expected_price(&f.p, signatures_required));
        f.settle(&attestation, args)
            .expect("settles at the computed price");
    }
}

#[test]
fn settlement_rejects_an_amount_that_is_not_the_computed_price() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let attestation = signed.attestation();
    let base = f.p.x402_args(&signed, [0xaa; 32]);

    for amount in [base.amount - 1, base.amount + 1, 0] {
        let args = SettleX402RoundArgs {
            amount,
            ..base.clone()
        };
        let failed = f.settle(&attestation, args).unwrap_err();
        assert_molpha_error(&failed, MolphaError::AmountMismatch);
    }
}

/// Pricing the threshold alone would leave every round short by the redundancy buffer, which
/// finalization is entitled to credit.
#[test]
fn the_price_covers_the_redundancy_buffer_not_just_the_threshold() {
    let mut f = fixture();
    let signatures_required = 4;
    let signed = f.p.sign_duty(duty(signatures_required));
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);

    let buffer = u64::from(f.p.args.redundancy_buffer);
    assert!(buffer > 0, "this fixture has a buffer to pay for");
    let threshold_only =
        f.p.args.x402_round_base + u64::from(signatures_required) * f.p.args.reward_per_signature;
    assert_eq!(
        args.amount,
        threshold_only + buffer * f.p.args.reward_per_signature
    );

    let failed = f
        .settle(
            &attestation,
            SettleX402RoundArgs {
                amount: threshold_only,
                ..args.clone()
            },
        )
        .unwrap_err();
    assert_molpha_error(&failed, MolphaError::AmountMismatch);
}

#[test]
fn payment_ref_is_accepted_at_both_length_boundaries_and_rejected_past_it() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let attestation = &signed.attestation();

    for (payer, len) in [([0x01u8; 32], 0usize), ([0x02u8; 32], 64)] {
        let args = SettleX402RoundArgs {
            payment_ref: vec![7u8; len],
            ..f.p.x402_args(&signed, payer)
        };
        f.settle(&attestation, args).unwrap_or_else(|e| {
            panic!("payment_ref of {len} bytes should be accepted: {:?}", e.err)
        });
    }

    let args = SettleX402RoundArgs {
        payment_ref: vec![7u8; 65],
        ..f.p.x402_args(&signed, [0x03; 32])
    };
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::PaymentRefTooLong);
}

#[test]
fn the_same_round_cannot_be_settled_twice_for_one_payer() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);

    f.settle(&attestation, args.clone())
        .expect("first settlement");
    let float_after_first = f.p.env.token_balance(&f.float_ata());

    let failed = f.settle(&attestation, args).unwrap_err();
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
    assert_eq!(
        f.p.env.token_balance(&f.float_ata()),
        float_after_first,
        "a blocked replay must not charge the float"
    );
}

#[test]
fn records_are_scoped_per_payer_so_two_payers_each_pay_in_full() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let attestation = signed.attestation();
    let first = f.p.x402_args(&signed, [0x11; 32]);
    let second = f.p.x402_args(&signed, [0x22; 32]);
    let price = first.amount;

    let float_before = f.p.env.token_balance(&f.float_ata());
    f.settle(&attestation, first.clone()).expect("first payer");
    f.settle(&attestation, second.clone())
        .expect("second payer");

    assert_ne!(
        f.record(&attestation, &first),
        f.record(&attestation, &second)
    );
    assert!(f.p.env.account_exists(&f.record(&attestation, &first)));
    assert!(f.p.env.account_exists(&f.record(&attestation, &second)));
    assert_eq!(
        f.p.env.token_balance(&f.float_ata()),
        float_before - 2 * price
    );
}

#[test]
fn settlement_rejects_an_underfunded_float() {
    let mut p = Protocol::start();
    p.register_nodes("x402-broke", NODE_COUNT);
    let gateway = p.register_gateway(0); // bonded, but no float
    let mut f = Fixture { p, gateway };

    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InsufficientGatewayFloat);
}

#[test]
fn the_bond_escrow_cannot_be_used_as_the_float() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);

    let mut ix =
        f.p.settle_x402_round_ix(&f.gateway.pubkey(), &attestation, args);
    let float = f.float_ata();
    let bond = f.bond_ata();
    for meta in ix.accounts.iter_mut() {
        if meta.pubkey == float {
            meta.pubkey = bond;
        }
    }
    let gateway = f.gateway.insecure_clone();
    let failed = f.p.env.send_err(&[ix], &[&gateway]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);
}

#[test]
fn a_third_partys_token_account_cannot_be_used_as_the_float() {
    let mut f = fixture();
    let stranger = f.p.env.keypair();
    let stranger_usdc = f.p.fund_usdc(&stranger.pubkey(), 10_000_000_000);

    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);
    let mut ix =
        f.p.settle_x402_round_ix(&f.gateway.pubkey(), &attestation, args);
    let float = f.float_ata();
    for meta in ix.accounts.iter_mut() {
        if meta.pubkey == float {
            meta.pubkey = stranger_usdc;
        }
    }
    let gateway = f.gateway.insecure_clone();
    let failed = f.p.env.send_err(&[ix], &[&gateway]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);
}

#[test]
fn settlement_requires_the_named_gateway_to_sign() {
    let mut f = fixture();
    let other_gateway = f.p.register_gateway(FLOAT);
    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);

    // Everything names the registered gateway (including the record PDA, which is seeded by it);
    // only the signing authority is swapped for a different gateway operator.
    let mut ix =
        f.p.settle_x402_round_ix(&f.gateway.pubkey(), &attestation, args);
    let registered_authority = f.gateway.pubkey();
    for meta in ix.accounts.iter_mut() {
        if meta.pubkey == registered_authority {
            meta.pubkey = other_gateway.pubkey();
        }
    }
    let failed = f.p.env.send_err(&[ix], &[&other_gateway]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
}

#[test]
fn settlement_rejects_a_quorum_below_the_protocol_minimum() {
    let mut f = fixture();
    assert_eq!(f.p.args.min_signers, 3);
    let signed = f.p.sign_duty(duty(2));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::SignaturesBelowMinSigners);
}

#[test]
fn settlement_rejects_signers_outside_the_derived_selection() {
    let mut f = fixture();
    let duty = duty(4);
    let selected = f.p.selection_bits(&duty);
    let unselected = (0..NODE_COUNT)
        .find(|bit| !selected.contains(bit))
        .expect("some node is outside the selection");
    let mut bits = selected[..3].to_vec();
    bits.push(unselected);
    bits.sort_unstable();

    let signed = f.p.sign_duty_with_bits(duty, &bits, None);
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
}

#[test]
fn settlement_rejects_a_quorum_below_signatures_required() {
    let mut f = fixture();
    let duty = duty(4);
    let selected = f.p.selection_bits(&duty);
    let signed = f.p.sign_duty_with_bits(duty, &selected[..3], None);
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);
}

#[test]
fn settlement_rejects_a_registry_after_its_grace_period() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let (attestation, args) = x402_settlement(&f.p, &signed, [0xaa; 32]);

    let authority = f.p.authority.insecure_clone();
    let next = f.p.current_version() + 1;
    let ix = f.p.update_registry_policy_ix(&authority.pubkey(), next, 4);
    f.p.env.send_ok(&[ix], &[&authority]);

    // advance the clock to after the grace period
    f.p.env
        .advance_seconds(f.p.args.max_settlement_delay_seconds as i64 + 1);

    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::RegistryNotActive);
}

#[test]
fn settlement_accepts_a_superseded_registry_during_grace() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty(4));
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);

    let authority = f.p.authority.insecure_clone();
    let next = f.p.current_version() + 1;
    let ix = f.p.update_registry_policy_ix(&authority.pubkey(), next, 4);
    f.p.env.send_ok(&[ix], &[&authority]);

    f.settle(&attestation, args).expect("settles during grace");
}

#[test]
fn a_settlement_leaves_the_gateway_bond_and_status_untouched() {
    let mut f = fixture();
    let bond_before = f.p.env.token_balance(&f.bond_ata());
    let signed = f.p.sign_duty(duty(4));
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);
    f.settle(&attestation, args).expect("settles");

    let after: Gateway = f.p.env.get(&f.gateway_pda());
    assert_eq!(f.p.env.token_balance(&f.bond_ata()), bond_before);
    assert_eq!(after.status, GatewayStatus::Active);
    assert_eq!(after.withdrawable_slot, 0);
}

#[test]
fn settlement_rejects_a_stale_canonical_timestamp() {
    let mut f = fixture();
    let now = f.p.env.unix_timestamp() as u64;
    let window = f.p.args.max_settlement_delay_seconds;

    // No consumer signature exists on this path, so the timestamp is pure gateway input;
    // the freshness window is what bounds the selection grind.
    let stale = duty(4).at(now - window - 1);
    let signed = f.p.sign_duty(stale);
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);

    // The window is inclusive.
    let boundary = duty(4).at(now - window);
    let signed = f.p.sign_duty(boundary);
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xab; 32]);
    f.settle(&attestation, args).expect("boundary settles");
}

#[test]
fn settlement_rejects_a_future_canonical_timestamp() {
    let mut f = fixture();
    let now = f.p.env.unix_timestamp() as u64;

    let future = duty(4).at(now + MAX_FUTURE_DRIFT_SECONDS + 1);
    let signed = f.p.sign_duty(future);
    let attestation = signed.attestation();
    let args = f.p.x402_args(&signed, [0xaa; 32]);
    let failed = f.settle(&attestation, args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::AttestationInFuture);
}
