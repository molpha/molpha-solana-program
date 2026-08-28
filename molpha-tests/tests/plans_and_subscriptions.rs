//! `add_plan`, `update_plan`, `subscribe`, `extend_subscription`.

use molpha::error::MolphaError;
use molpha::events::{PlanAdded, PlanUpdated, Subscribed, SubscriptionExtended};
use molpha::instructions::{AddPlanArgs, UpdatePlanArgs};
use molpha::state::{Plan, PlanType, Subscription};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

fn plan_args(plan_type: PlanType) -> AddPlanArgs {
    AddPlanArgs {
        plan_type,
        subscription_price: 25_000_000,
        max_signers: 8,
        max_delegates: 2,
        max_rounds: 40,
        private_api_enabled: true,
        is_active: true,
    }
}

#[test]
fn add_plan_stores_every_field_and_emits_the_plan() {
    let mut p = Protocol::start();
    let args = plan_args(PlanType::Professional);
    let meta = p.add_plan(args.clone());

    let plan: Plan = p.env.get(&pda::plan(PlanType::Professional));
    assert_eq!(plan.plan_type, PlanType::Professional);
    assert_eq!(plan.subscription_price, args.subscription_price);
    assert_eq!(plan.max_signers, args.max_signers);
    assert_eq!(plan.max_delegates, args.max_delegates);
    assert_eq!(plan.max_rounds, args.max_rounds);
    assert!(plan.private_api_enabled);
    assert!(plan.is_active);

    let event: PlanAdded = only_event(&meta);
    assert_eq!(event.plan_type, PlanType::Professional as u8);
    assert_eq!(event.subscription_price, args.subscription_price);
}

#[test]
fn each_plan_type_gets_its_own_account() {
    let mut p = Protocol::start();
    for plan_type in [
        PlanType::Basic,
        PlanType::Standard,
        PlanType::Professional,
        PlanType::Enterprise,
    ] {
        p.add_plan(AddPlanArgs {
            subscription_price: 1_000_000 * (plan_type as u64 + 1),
            ..plan_args(plan_type)
        });
    }
    for plan_type in [
        PlanType::Basic,
        PlanType::Standard,
        PlanType::Professional,
        PlanType::Enterprise,
    ] {
        let plan: Plan = p.env.get(&pda::plan(plan_type));
        assert_eq!(plan.plan_type, plan_type);
        assert_eq!(plan.subscription_price, 1_000_000 * (plan_type as u64 + 1));
    }
}

#[test]
fn add_plan_rejects_a_caller_that_is_not_the_authority() {
    let mut p = Protocol::start();
    let impostor = p.env.keypair();
    let ix = p.add_plan_ix(&plan_args(PlanType::Basic), &impostor.pubkey());
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
    assert!(!p.env.account_exists(&pda::plan(PlanType::Basic)));
}

#[test]
fn add_plan_cannot_overwrite_an_existing_plan() {
    let mut p = Protocol::start();
    p.add_plan(plan_args(PlanType::Basic));

    let authority = p.authority.insecure_clone();
    let ix = p.add_plan_ix(
        &AddPlanArgs {
            subscription_price: 1,
            ..plan_args(PlanType::Basic)
        },
        &authority.pubkey(),
    );
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );

    let plan: Plan = p.env.get(&pda::plan(PlanType::Basic));
    assert_eq!(plan.subscription_price, 25_000_000);
}

#[test]
fn update_plan_rewrites_limits_and_can_retire_a_plan() {
    let mut p = Protocol::start();
    p.add_plan(plan_args(PlanType::Standard));

    let args = UpdatePlanArgs {
        plan_type: PlanType::Standard,
        subscription_price: 99_000_000,
        max_signers: 20,
        max_delegates: 9,
        max_rounds: 1_000,
        private_api_enabled: false,
        is_active: false,
    };
    let authority = p.authority.insecure_clone();
    let ix = p.update_plan_ix(&authority.pubkey(), &args);
    let meta = p.env.send_ok(&[ix], &[&authority]);

    let plan: Plan = p.env.get(&pda::plan(PlanType::Standard));
    assert_eq!(plan.subscription_price, args.subscription_price);
    assert_eq!(plan.max_signers, args.max_signers);
    assert_eq!(plan.max_delegates, args.max_delegates);
    assert_eq!(plan.max_rounds, args.max_rounds);
    assert!(!plan.private_api_enabled);
    assert!(!plan.is_active);
    // `plan_type` is the PDA seed and is never rewritten.
    assert_eq!(plan.plan_type, PlanType::Standard);

    let event: PlanUpdated = only_event(&meta);
    assert_eq!(event.subscription_price, args.subscription_price);
}

#[test]
fn update_plan_rejects_a_caller_that_is_not_the_authority() {
    let mut p = Protocol::start();
    p.add_plan(plan_args(PlanType::Standard));
    let impostor = p.env.keypair();
    let args = UpdatePlanArgs {
        plan_type: PlanType::Standard,
        subscription_price: 1,
        max_signers: 1,
        max_delegates: 1,
        max_rounds: 1,
        private_api_enabled: false,
        is_active: false,
    };
    let ix = p.update_plan_ix(&impostor.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);

    let plan: Plan = p.env.get(&pda::plan(PlanType::Standard));
    assert_eq!(plan.subscription_price, 25_000_000);
}

// ---------------------------------------------------------------------------
// subscribe
// ---------------------------------------------------------------------------

#[test]
fn subscribe_charges_the_plan_price_and_snapshots_its_limits() {
    let mut p = Protocol::start();
    let plan = p.add_default_plan(PlanType::Professional);

    let owner = p.env.keypair();
    let owner_usdc = p.fund_usdc(&owner.pubkey(), 500_000_000);
    let before_owner = p.env.token_balance(&owner_usdc);
    let before_treasury = p.treasury_balance();
    let now = p.env.unix_timestamp();

    let ix = p.subscribe_ix(&owner.pubkey(), PlanType::Professional);
    let meta = p.env.send_ok(&[ix], &[&owner]);

    let subscription: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.owner, owner.pubkey());
    assert_eq!(subscription.plan_type, PlanType::Professional);
    assert_eq!(subscription.prepaid_usdc, plan.subscription_price);
    assert_eq!(subscription.price, plan.subscription_price);
    assert_eq!(subscription.used_rounds, 0);
    assert_eq!(subscription.max_rounds, plan.max_rounds);
    assert_eq!(subscription.delegate_count, 0);
    assert_eq!(subscription.max_delegates, plan.max_delegates);
    assert_eq!(subscription.max_signers, plan.max_signers);
    assert_eq!(
        subscription.valid_until,
        now + Subscription::SECONDS_PER_MONTH
    );

    assert_eq!(
        p.env.token_balance(&owner_usdc),
        before_owner - plan.subscription_price
    );
    assert_eq!(
        p.treasury_balance(),
        before_treasury + plan.subscription_price
    );

    let event: Subscribed = only_event(&meta);
    assert_eq!(event.owner, owner.pubkey());
    assert_eq!(event.prepaid_usdc, plan.subscription_price);
    assert_eq!(event.valid_until, subscription.valid_until);
}

#[test]
fn subscribe_rejects_an_inactive_plan() {
    let mut p = Protocol::start();
    p.add_plan(AddPlanArgs {
        is_active: false,
        ..plan_args(PlanType::Basic)
    });

    let owner = p.env.keypair();
    p.fund_usdc(&owner.pubkey(), 500_000_000);
    let ix = p.subscribe_ix(&owner.pubkey(), PlanType::Basic);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::PlanNotActive);
}

#[test]
fn subscribe_rejects_a_plan_that_was_never_added() {
    let mut p = Protocol::start();
    let owner = p.env.keypair();
    p.fund_usdc(&owner.pubkey(), 500_000_000);
    let ix = p.subscribe_ix(&owner.pubkey(), PlanType::Enterprise);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
}

#[test]
fn subscribe_is_one_subscription_per_owner() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);

    let ix = p.subscribe_ix(&owner.pubkey(), PlanType::Basic);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn subscribe_rejects_an_owner_who_cannot_pay() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.env.keypair();
    // ATA exists but holds less than the plan price.
    p.fund_usdc(&owner.pubkey(), 1);

    let ix = p.subscribe_ix(&owner.pubkey(), PlanType::Basic);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("insufficient funds")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
    assert!(!p.env.account_exists(&pda::subscription(&owner.pubkey())));
}

// ---------------------------------------------------------------------------
// extend_subscription
// ---------------------------------------------------------------------------

#[test]
fn extend_adds_a_month_to_a_still_valid_subscription() {
    let mut p = Protocol::start();
    let plan = p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);
    let before: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));

    p.env.advance_seconds(60);
    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic);
    let meta = p.env.send_ok(&[ix], &[&owner]);

    let after: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(
        after.valid_until,
        before.valid_until + Subscription::SECONDS_PER_MONTH,
        "an active subscription extends from its own expiry, not from now"
    );
    assert_eq!(
        after.prepaid_usdc,
        before.prepaid_usdc + plan.subscription_price
    );
    assert_eq!(after.price, plan.subscription_price);
    assert_eq!(after.used_rounds, before.used_rounds);

    let event: SubscriptionExtended = only_event(&meta);
    assert_eq!(event.valid_until, after.valid_until);
    assert_eq!(event.prepaid_usdc, after.prepaid_usdc);
}

#[test]
fn extend_restarts_the_month_from_now_once_the_subscription_lapsed() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);

    p.env
        .advance_seconds(Subscription::SECONDS_PER_MONTH + 10_000);
    let now = p.env.unix_timestamp();
    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic);
    p.env.send_ok(&[ix], &[&owner]);

    let after: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(after.valid_until, now + Subscription::SECONDS_PER_MONTH);
}

#[test]
fn extend_charges_the_current_plan_price_not_the_snapshot() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);
    let owner_usdc = pda::ata(&owner.pubkey(), &p.usdc_mint);

    // Re-price the plan after the owner subscribed.
    let authority = p.authority.insecure_clone();
    let args = UpdatePlanArgs {
        plan_type: PlanType::Basic,
        subscription_price: 7_000_000,
        max_signers: 16,
        max_delegates: 4,
        max_rounds: 100,
        private_api_enabled: true,
        is_active: true,
    };
    let ix = p.update_plan_ix(&authority.pubkey(), &args);
    p.env.send_ok(&[ix], &[&authority]);

    let before_owner = p.env.token_balance(&owner_usdc);
    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic);
    p.env.send_ok(&[ix], &[&owner]);

    assert_eq!(p.env.token_balance(&owner_usdc), before_owner - 7_000_000);
    let after: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(after.price, 7_000_000);
}

#[test]
fn extend_rejects_a_retired_plan() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);

    let authority = p.authority.insecure_clone();
    let args = UpdatePlanArgs {
        plan_type: PlanType::Basic,
        subscription_price: 50_000_000,
        max_signers: 16,
        max_delegates: 4,
        max_rounds: 100,
        private_api_enabled: true,
        is_active: false,
    };
    let ix = p.update_plan_ix(&authority.pubkey(), &args);
    p.env.send_ok(&[ix], &[&authority]);

    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::PlanNotActive);
}

#[test]
fn extend_rejects_a_plan_account_that_is_not_the_subscribed_one() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    p.add_default_plan(PlanType::Enterprise);
    let owner = p.subscribe(PlanType::Basic);

    // The plan seed is checked against `subscription.plan_type`, not the caller's choice.
    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Enterprise);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}
