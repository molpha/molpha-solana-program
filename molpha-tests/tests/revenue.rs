//! Income: x402 payments and subscriptions land in the treasury and are swept into the open
//! epoch's pool; the protocol keeps `protocol_fee_bps` as a withdrawable reserve.

use anchor_lang::error::ErrorCode;
use molpha::error::MolphaError;
use molpha::events::RevenueSwept;
use molpha::state::PlanType;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

fn bps(amount: u64, bps: u16) -> u64 {
    amount * u64::from(bps) / 10_000
}

/// x402 payers pay the treasury directly; the sweep only books the split, no tokens move.
#[test]
fn a_sweep_splits_unswept_treasury_income_between_pool_and_protocol() {
    let mut p = Protocol::start();
    let income = 1_000_000;
    p.fund_treasury(income);

    let meta = p.env.send_ok(&[p.sweep_treasury_revenue_ix(0)], &[]);

    let protocol_fee = bps(income, p.args.protocol_fee_bps);
    let pool = income - protocol_fee;
    assert_eq!(p.env.epoch(0).pool, pool);
    assert_eq!(p.treasury_balance(), income);
    let config = p.config();
    assert_eq!(config.pool_liability, pool);
    assert_eq!(config.protocol_reserved, protocol_fee);

    let event: RevenueSwept = only_event(&meta);
    assert_eq!(
        (
            event.epoch,
            event.gross,
            event.protocol_fee,
            event.pool_share
        ),
        (0, income, protocol_fee, pool)
    );
    p.assert_solvent();
}

#[test]
fn rounding_favours_the_pool() {
    let mut p = Protocol::start();
    p.earn_income(999);
    // 5% of 999 floors to 49.
    assert_eq!(p.env.epoch(0).pool, 999 - 49);
    p.assert_solvent();
}

#[test]
fn a_zero_protocol_fee_sends_all_income_to_the_pool() {
    let mut p = Protocol::builder().protocol_fee_bps(0).start();
    p.earn_income(1_000_000);
    assert_eq!(p.env.epoch(0).pool, 1_000_000);
    assert_eq!(p.config().protocol_reserved, 0);
    p.assert_solvent();
}

#[test]
fn income_is_recognized_once_and_committed_funds_are_never_swept() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    p.subscribe(PlanType::Basic);
    let price = 50_000_000;
    assert_eq!(p.treasury_balance(), price);

    p.env.send_ok(&[p.sweep_treasury_revenue_ix(0)], &[]);
    let protocol_fee = bps(price, p.args.protocol_fee_bps);
    assert_eq!(p.env.epoch(0).pool, price - protocol_fee);
    assert_eq!(p.config().protocol_reserved, protocol_fee);

    // Everything in the treasury is now committed: pool plus protocol reserve.
    let failed = p.env.send_err(&[p.sweep_treasury_revenue_ix(0)], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToSweep);

    // A later x402 payment is swept on its own.
    p.earn_income(1_000);
    assert_eq!(
        p.env.epoch(0).pool,
        price - protocol_fee + 1_000 - bps(1_000, p.args.protocol_fee_bps)
    );
    p.assert_solvent();
}

#[test]
fn sweeps_credit_only_the_epoch_open_now() {
    let mut p = Protocol::start();
    p.fund_treasury(1_000);

    let failed = p.env.send_err(&[p.sweep_treasury_revenue_ix(1)], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotOpen);

    // In epoch 0's ticket grace, income already belongs to epoch 1.
    let e0 = p.env.epoch(0);
    p.env.warp_to(e0.end);
    let failed = p.env.send_err(&[p.sweep_treasury_revenue_ix(0)], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotOpen);
    p.env.send_ok(&[p.sweep_treasury_revenue_ix(1)], &[]);
    assert_eq!(p.env.epoch(0).pool, 0);
    assert!(p.env.epoch(1).pool > 0);
}

#[test]
fn the_protocol_reserve_is_withdrawable_only_by_the_authority_and_only_up_to_the_reserve() {
    let mut p = Protocol::start();
    p.earn_income(1_000_000);
    let reserve = p.config().protocol_reserved;
    let authority = p.authority.insecure_clone();
    let destination = p.fund_usdc(&authority.pubkey(), 0);

    let stranger = p.env.keypair();
    let ix = p.withdraw_protocol_reserve_ix(&stranger.pubkey(), &destination, 1);
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_anchor_error(&failed, ErrorCode::ConstraintHasOne);

    let ix = p.withdraw_protocol_reserve_ix(&authority.pubkey(), &destination, reserve + 1);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InsufficientProtocolReserve);

    let ix = p.withdraw_protocol_reserve_ix(&authority.pubkey(), &destination, reserve);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(p.env.token_balance(&destination), reserve);
    assert_eq!(p.config().protocol_reserved, 0);

    // Withdrawing committed funds must not surface as fresh income.
    let failed = p.env.send_err(&[p.sweep_treasury_revenue_ix(0)], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToSweep);
    p.assert_solvent();
}
