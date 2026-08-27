//! `add_delegate`, `remove_delegate`, `set_delegate_paused`.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{DelegateAdded, DelegateRemoved, DelegateStatusChanged};
use molpha::instructions::{AddDelegateArgs, AddPlanArgs};
use molpha::state::{AgentStatus, Delegate, PlanType, Subscription};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// A delegate authority key. Delegates never sign here, so a bare pubkey is enough.
fn delegate_key() -> Pubkey {
    Pubkey::new_unique()
}

/// A protocol with one subscriber on a plan that allows `max_delegates` delegates.
fn subscribed(max_delegates: u32, max_signers: u8) -> (Protocol, Keypair) {
    let mut p = Protocol::start();
    p.add_plan(AddPlanArgs {
        plan_type: PlanType::Standard,
        subscription_price: 10_000_000,
        max_signers,
        max_delegates,
        max_rounds: 100,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = p.subscribe(PlanType::Standard);
    (p, owner)
}

#[test]
fn add_delegate_writes_the_grant_and_counts_it_against_the_subscription() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();

    let args = AddDelegateArgs {
        delegate,
        max_data_requests: 25,
        max_signers: 6,
    };
    let ix = p.add_delegate_ix(&owner.pubkey(), &args);
    let meta = p.env.send_ok(&[ix], &[&owner]);

    let account: Delegate = p.env.get(&pda::delegate(&owner.pubkey(), &delegate));
    assert_eq!(account.owner, owner.pubkey());
    assert_eq!(account.delegate, delegate);
    assert_eq!(account.status, AgentStatus::Active);
    assert_eq!(account.max_data_requests, 25);
    assert_eq!(account.data_requests_used, 0);
    assert_eq!(account.max_signers, 6);

    let subscription: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.delegate_count, 1);

    let event: DelegateAdded = only_event(&meta);
    assert_eq!(event.delegate, delegate);
    assert_eq!(event.delegate_count, 1);
}

#[test]
fn add_delegate_allows_max_signers_up_to_but_not_above_the_subscription_ceiling() {
    let (mut p, owner) = subscribed(4, 8);

    // Exactly at the ceiling is allowed.
    let at_ceiling = delegate_key();
    p.add_delegate(&owner, &at_ceiling, 5, 8);
    let account: Delegate = p.env.get(&pda::delegate(&owner.pubkey(), &at_ceiling));
    assert_eq!(account.max_signers, 8);

    // One above is not — the program rejects rather than clamping.
    let above = delegate_key();
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate: above,
            max_data_requests: 5,
            max_signers: 9,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::SignaturesExceedSubscriptionMax);
    assert!(!p
        .env
        .account_exists(&pda::delegate(&owner.pubkey(), &above)));
}

#[test]
fn add_delegate_rejects_zero_max_signers() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate,
            max_data_requests: 5,
            max_signers: 0,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::SignaturesExceedSubscriptionMax);
}

#[test]
fn add_delegate_rejects_a_zero_request_quota() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate,
            max_data_requests: 0,
            max_signers: 4,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidDelegateLimits);
}

#[test]
fn add_delegate_rejects_the_owner_delegating_to_itself() {
    let (mut p, owner) = subscribed(2, 8);
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate: owner.pubkey(),
            max_data_requests: 5,
            max_signers: 4,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::DelegateCannotBeOwner);
}

#[test]
fn add_delegate_enforces_the_plans_delegate_cap() {
    let (mut p, owner) = subscribed(2, 8);
    p.add_delegate(&owner, &delegate_key(), 5, 4);
    p.add_delegate(&owner, &delegate_key(), 5, 4);

    let third = delegate_key();
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate: third,
            max_data_requests: 5,
            max_signers: 4,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::DelegateCapReached);

    let subscription: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.delegate_count, 2);
}

#[test]
fn add_delegate_rejects_an_expired_subscription() {
    let (mut p, owner) = subscribed(2, 8);
    p.env.advance_seconds(Subscription::SECONDS_PER_MONTH + 1);

    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate: delegate_key(),
            max_data_requests: 5,
            max_signers: 4,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::SubscriptionNotActive);
}

#[test]
fn add_delegate_requires_a_subscription() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let stranger = p.env.keypair();

    let ix = p.add_delegate_ix(
        &stranger.pubkey(),
        &AddDelegateArgs {
            delegate: delegate_key(),
            max_data_requests: 5,
            max_signers: 4,
        },
    );
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
}

#[test]
fn a_delegate_grant_is_scoped_to_one_owner() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    p.add_delegate(&owner, &delegate, 5, 4);

    // A second subscriber may grant the same delegate key; the PDAs are distinct.
    let other = p.subscribe(PlanType::Standard);
    p.add_delegate(&other, &delegate, 3, 2);

    let first: Delegate = p.env.get(&pda::delegate(&owner.pubkey(), &delegate));
    let second: Delegate = p.env.get(&pda::delegate(&other.pubkey(), &delegate));
    assert_eq!(first.owner, owner.pubkey());
    assert_eq!(second.owner, other.pubkey());
    assert_eq!(first.max_data_requests, 5);
    assert_eq!(second.max_data_requests, 3);
}

#[test]
fn remove_delegate_closes_the_account_and_frees_a_slot() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    p.add_delegate(&owner, &delegate, 5, 4);

    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate);
    let rent = p.env.lamports(&delegate_pda);
    assert!(rent > 0);
    let owner_lamports_before = p.env.lamports(&owner.pubkey());

    let ix = p.remove_delegate_ix(&owner.pubkey(), &delegate);
    let meta = p.env.send_ok(&[ix], &[&owner]);

    assert!(!p.env.account_exists(&delegate_pda));
    assert_eq!(
        p.env.lamports(&owner.pubkey()),
        owner_lamports_before + rent,
        "closing refunds rent to the owner"
    );

    let subscription: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.delegate_count, 0);

    let event: DelegateRemoved = only_event(&meta);
    assert_eq!(event.delegate, delegate);
    assert_eq!(event.delegate_count, 0);

    // The freed slot is reusable.
    p.add_delegate(&owner, &delegate_key(), 5, 4);
    p.add_delegate(&owner, &delegate_key(), 5, 4);
}

#[test]
fn remove_delegate_rejects_a_caller_who_is_not_the_granting_owner() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    p.add_delegate(&owner, &delegate, 5, 4);

    let impostor = p.env.keypair();
    // Seeds are owner-scoped, so an impostor cannot even name the account.
    let ix = p.remove_delegate_ix(&impostor.pubkey(), &delegate);
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
    assert!(p
        .env
        .account_exists(&pda::delegate(&owner.pubkey(), &delegate)));
}

#[test]
fn set_delegate_paused_toggles_status_in_both_directions() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    p.add_delegate(&owner, &delegate, 5, 4);
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate);

    let ix = p.set_delegate_paused_ix(&owner.pubkey(), &delegate, true);
    let meta = p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(
        p.env.get::<Delegate>(&delegate_pda).status,
        AgentStatus::Paused
    );
    let event: DelegateStatusChanged = only_event(&meta);
    assert_eq!(event.old_status, AgentStatus::Active as u8);
    assert_eq!(event.new_status, AgentStatus::Paused as u8);

    let ix = p.set_delegate_paused_ix(&owner.pubkey(), &delegate, false);
    p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(
        p.env.get::<Delegate>(&delegate_pda).status,
        AgentStatus::Active
    );
}

#[test]
fn set_delegate_paused_rejects_a_caller_who_is_not_the_granting_owner() {
    let (mut p, owner) = subscribed(2, 8);
    let delegate = delegate_key();
    p.add_delegate(&owner, &delegate, 5, 4);

    let impostor = p.env.keypair();
    let ix = p.set_delegate_paused_ix(&impostor.pubkey(), &delegate, true);
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
    assert_eq!(
        p.env
            .get::<Delegate>(&pda::delegate(&owner.pubkey(), &delegate))
            .status,
        AgentStatus::Active
    );
}
