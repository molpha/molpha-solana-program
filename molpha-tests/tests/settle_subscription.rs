//! `settle_subscription_round`: consumer authorization, quotas, and the round record.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use molpha::error::MolphaError;
use molpha::events::RoundSettled;
use molpha::instructions::AddPlanArgs;
use molpha::state::{
    Delegate, PlanType, RoundReceiptRecord, RoundRecordKind, RoundRecordStatus, Subscription,
};
use molpha_tests::env::{
    assert_anchor_error, assert_molpha_error, only_event, set_compute_unit_price,
};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const PLAN_MAX_SIGNERS: u8 = 6;
const PLAN_MAX_ROUNDS: u64 = 3;

/// A protocol with nodes, a bonded gateway and one subscriber.
struct Fixture {
    p: Protocol,
    gateway: Keypair,
    owner: Keypair,
}

fn fixture() -> Fixture {
    fixture_with_rounds(PLAN_MAX_ROUNDS)
}

fn fixture_with_rounds(max_rounds: u64) -> Fixture {
    let mut p = Protocol::start();
    p.register_nodes("settle-sub", NODE_COUNT);
    p.add_plan(AddPlanArgs {
        plan_type: PlanType::Professional,
        subscription_price: 10_000_000,
        max_signers: PLAN_MAX_SIGNERS,
        max_delegates: 2,
        max_rounds,
        private_api_enabled: true,
        is_active: true,
    });
    let owner = p.subscribe(PlanType::Professional);
    let gateway = p.register_gateway(0);
    Fixture { p, gateway, owner }
}

fn duty(signatures_required: u8) -> Duty {
    Duty::new(NODE_COUNT as u32, signatures_required)
}

/// `[ed25519 consumer authorization, settle_subscription_round]`.
fn settle_ixs(
    f: &Fixture,
    consumer: &Keypair,
    delegate_account: Option<Pubkey>,
    signed: &SignedAttestation,
) -> Vec<Instruction> {
    vec![
        f.p.consumer_authorization_ix(consumer, &f.gateway.pubkey(), &signed.duty),
        f.p.settle_subscription_round_ix(
            &f.gateway.pubkey(),
            &f.owner.pubkey(),
            &consumer.pubkey(),
            delegate_account,
            signed,
        ),
    ]
}

fn round_record(f: &Fixture, duty: &Duty) -> Pubkey {
    pda::subscription_round(
        &f.owner.pubkey(),
        &pda::gateway(&f.gateway.pubkey()),
        &duty.source_id,
        duty.signatures_required,
        duty.canonical_timestamp,
    )
}

// ---------------------------------------------------------------------------
// happy paths
// ---------------------------------------------------------------------------

#[test]
fn owner_direct_settlement_records_the_round_and_consumes_quota() {
    let mut f = fixture();
    let duty = duty(4);
    let signed = f.p.sign_duty(duty);

    let treasury_before = f.p.treasury_balance();
    let ixs = settle_ixs(&f, &f.owner, None, &signed);
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let meta = f.p.env.send_ok(&ixs, &[&owner, &gateway]);

    let record: RoundReceiptRecord = f.p.env.get(&round_record(&f, &duty));
    assert_eq!(record.status, RoundRecordStatus::Pending);
    assert_eq!(record.kind, RoundRecordKind::Subscription);
    assert_eq!(record.gateway, pda::gateway(&gateway.pubkey()));
    assert_eq!(record.registry_version, duty.registry_version);
    assert_eq!(record.signers_bitmap, signed.signers_bitmap);
    assert_eq!(record.agg_sig_s, signed.signature.agg_sig_s);
    assert_eq!(record.commitment, signed.signature.commitment);
    assert_eq!(record.attestation_message_hash, signed.message_hash);
    assert_eq!(record.reward_per_signature, f.p.args.reward_per_signature);
    assert_eq!(record.recorded_slot, f.p.env.slot());

    let subscription: Subscription = f.p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.used_rounds, 1);

    // Settlement is pure accounting; nothing moves until `finalize_round`.
    assert_eq!(f.p.treasury_balance(), treasury_before);

    let event: RoundSettled = only_event(&meta);
    assert_eq!(event.gateway, pda::gateway(&gateway.pubkey()));
    assert_eq!(event.source_id, duty.source_id);
    assert_eq!(event.canonical_timestamp, duty.canonical_timestamp);
    assert_eq!(
        event.credited_count,
        signed
            .signers_bitmap
            .iter()
            .map(|b| b.count_ones())
            .sum::<u32>()
    );
}

#[test]
fn delegate_settlement_consumes_the_delegates_request_quota() {
    let mut f = fixture();
    let delegate = f.p.env.keypair();
    let owner = f.owner.insecure_clone();
    f.p.add_delegate(&owner, &delegate.pubkey(), 5, 4);

    let duty = duty(4);
    let signed = f.p.sign_duty(duty);
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate.pubkey());
    let ixs = settle_ixs(&f, &delegate, Some(delegate_pda), &signed);
    let gateway = f.gateway.insecure_clone();
    f.p.env.send_ok(&ixs, &[&delegate, &gateway]);

    let account: Delegate = f.p.env.get(&delegate_pda);
    assert_eq!(account.data_requests_used, 1);
    let subscription: Subscription = f.p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.used_rounds, 1);
    assert!(f.p.env.account_exists(&round_record(&f, &duty)));
}

#[test]
fn distinct_rounds_get_distinct_records() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();

    let first = duty(4);
    let second = first.at(first.canonical_timestamp + 1);
    for d in [first, second] {
        let signed = f.p.sign_duty(d);
        let ixs = settle_ixs(&f, &owner, None, &signed);
        f.p.env.send_ok(&ixs, &[&owner, &gateway]);
    }

    assert_ne!(round_record(&f, &first), round_record(&f, &second));
    assert!(f.p.env.account_exists(&round_record(&f, &first)));
    assert!(f.p.env.account_exists(&round_record(&f, &second)));
    let subscription: Subscription = f.p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.used_rounds, 2);
}

#[test]
fn the_same_round_cannot_be_settled_twice() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let duty = duty(4);
    let signed = f.p.sign_duty(duty);

    let ixs = settle_ixs(&f, &owner, None, &signed);
    f.p.env.send_ok(&ixs, &[&owner, &gateway]);

    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );

    let subscription: Subscription = f.p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(
        subscription.used_rounds, 1,
        "the replay must not burn quota"
    );
}

// ---------------------------------------------------------------------------
// consumer authorization
// ---------------------------------------------------------------------------

#[test]
fn settlement_requires_a_consumer_ed25519_signature() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let signed = f.p.sign_duty(duty(4));

    let ix = f.p.settle_subscription_round_ix(
        &gateway.pubkey(),
        &owner.pubkey(),
        &owner.pubkey(),
        None,
        &signed,
    );
    let failed = f.p.env.send_err(&[ix], &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::ConsumerSigInvalid);
}

#[test]
fn settlement_rejects_an_authorization_signed_by_someone_else() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let impostor = f.p.env.keypair();
    let signed = f.p.sign_duty(duty(4));

    // Ed25519 instruction is present and internally valid, but for the wrong pubkey.
    let ixs = vec![
        f.p.consumer_authorization_ix(&impostor, &gateway.pubkey(), &signed.duty),
        f.p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::ConsumerSigInvalid);
}

#[test]
fn settlement_rejects_an_authorization_over_a_different_request() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let duty = duty(4);
    let signed = f.p.sign_duty(duty);

    // Signed over a neighbouring timestamp: the request-auth hash no longer matches.
    let other_request = duty.at(duty.canonical_timestamp + 1);
    let ixs = vec![
        f.p.consumer_authorization_ix(&owner, &gateway.pubkey(), &other_request),
        f.p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::ConsumerSigInvalid);
}

#[test]
fn an_authorization_naming_one_gateway_cannot_be_redeemed_by_another() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let other_gateway = f.p.register_gateway(0);
    let signed = f.p.sign_duty(duty(4));

    // The consumer authorized the fixture gateway. `other_gateway` lifts that plaintext Ed25519
    // instruction out of the transaction and settles the same round under its own gateway account:
    // the preimage commits to the chosen gateway, so the signature no longer matches.
    let ixs = vec![
        f.p.consumer_authorization_ix(&owner, &f.gateway.pubkey(), &signed.duty),
        f.p.settle_subscription_round_ix(
            &other_gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    let failed = f.p.env.send_err(&ixs, &[&owner, &other_gateway]);
    assert_molpha_error(&failed, MolphaError::ConsumerSigInvalid);
}

#[test]
fn the_authorization_must_be_the_immediately_preceding_instruction() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let signed = f.p.sign_duty(duty(4));

    let ixs = vec![
        f.p.consumer_authorization_ix(&owner, &gateway.pubkey(), &signed.duty),
        // Anything wedged in between breaks the introspection contract.
        set_compute_unit_price(1),
        f.p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &owner.pubkey(),
            &owner.pubkey(),
            None,
            &signed,
        ),
    ];
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::ConsumerSigInvalid);
}

#[test]
fn owner_direct_settlement_rejects_a_delegate_account() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let delegate = f.p.env.keypair();
    f.p.add_delegate(&owner, &delegate.pubkey(), 5, 4);

    let signed = f.p.sign_duty(duty(4));
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate.pubkey());
    let ixs = settle_ixs(&f, &owner, Some(delegate_pda), &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::UnexpectedDelegateAccount);
}

#[test]
fn delegate_settlement_requires_the_delegate_account() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let delegate = f.p.env.keypair();
    f.p.add_delegate(&owner, &delegate.pubkey(), 5, 4);

    let signed = f.p.sign_duty(duty(4));
    let ixs = settle_ixs(&f, &delegate, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&delegate, &gateway]);
    assert_molpha_error(&failed, MolphaError::DelegateAccountRequired);
}

#[test]
fn a_delegate_of_another_owner_cannot_spend_this_subscription() {
    let mut f = fixture();
    let gateway = f.gateway.insecure_clone();
    let other_owner = f.p.subscribe(PlanType::Professional);
    let delegate = f.p.env.keypair();
    f.p.add_delegate(&other_owner, &delegate.pubkey(), 5, 4);

    let signed = f.p.sign_duty(duty(4));
    // The delegate is real, but it belongs to `other_owner`'s subscription.
    let foreign_delegate_pda = pda::delegate(&other_owner.pubkey(), &delegate.pubkey());
    let ixs = settle_ixs(&f, &delegate, Some(foreign_delegate_pda), &signed);
    let failed = f.p.env.send_err(&ixs, &[&delegate, &gateway]);
    assert_molpha_error(&failed, MolphaError::UnauthorisedConsumer);
}

#[test]
fn a_paused_delegate_cannot_settle() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let delegate = f.p.env.keypair();
    f.p.add_delegate(&owner, &delegate.pubkey(), 5, 4);
    let ix =
        f.p.set_delegate_paused_ix(&owner.pubkey(), &delegate.pubkey(), true);
    f.p.env.send_ok(&[ix], &[&owner]);

    let signed = f.p.sign_duty(duty(4));
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate.pubkey());
    let ixs = settle_ixs(&f, &delegate, Some(delegate_pda), &signed);
    let failed = f.p.env.send_err(&ixs, &[&delegate, &gateway]);
    assert_molpha_error(&failed, MolphaError::DelegateNotActive);
}

#[test]
fn a_delegate_runs_out_of_data_requests() {
    let mut f = fixture_with_rounds(10);
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let delegate = f.p.env.keypair();
    f.p.add_delegate(&owner, &delegate.pubkey(), 2, 4);
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate.pubkey());

    let base = duty(4);
    for offset in 0..2u64 {
        let signed = f.p.sign_duty(base.at(base.canonical_timestamp + offset));
        let ixs = settle_ixs(&f, &delegate, Some(delegate_pda), &signed);
        f.p.env.send_ok(&ixs, &[&delegate, &gateway]);
    }

    let signed = f.p.sign_duty(base.at(base.canonical_timestamp + 2));
    let ixs = settle_ixs(&f, &delegate, Some(delegate_pda), &signed);
    let failed = f.p.env.send_err(&ixs, &[&delegate, &gateway]);
    assert_molpha_error(&failed, MolphaError::DelegateDataRequestLimitReached);

    let account: Delegate = f.p.env.get(&delegate_pda);
    assert_eq!(account.data_requests_used, 2);
}

// ---------------------------------------------------------------------------
// quorum limits
// ---------------------------------------------------------------------------

#[test]
fn settlement_rejects_a_quorum_below_the_protocol_minimum() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    assert_eq!(f.p.args.min_signers, 3);

    let signed = f.p.sign_duty(duty(2));
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::SignaturesBelowMinSigners);
}

#[test]
fn owner_direct_settlement_is_capped_by_the_subscription_ceiling() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();

    // Exactly at the ceiling is allowed.
    let signed = f.p.sign_duty(duty(PLAN_MAX_SIGNERS));
    let ixs = settle_ixs(&f, &owner, None, &signed);
    f.p.env.send_ok(&ixs, &[&owner, &gateway]);

    // One above is not.
    let signed = f.p.sign_duty(duty(PLAN_MAX_SIGNERS + 1));
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::SignaturesExceedSubscriptionMax);
}

#[test]
fn delegate_settlement_is_capped_by_the_delegate_ceiling() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let delegate = f.p.env.keypair();
    // The delegate is granted less than the subscription allows.
    f.p.add_delegate(&owner, &delegate.pubkey(), 5, 4);
    let delegate_pda = pda::delegate(&owner.pubkey(), &delegate.pubkey());

    let signed = f.p.sign_duty(duty(5));
    let ixs = settle_ixs(&f, &delegate, Some(delegate_pda), &signed);
    let failed = f.p.env.send_err(&ixs, &[&delegate, &gateway]);
    assert_molpha_error(&failed, MolphaError::SignaturesExceedDelegateMax);
}

#[test]
fn settlement_rejects_signers_outside_the_derived_selection() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let duty = duty(4);

    let selected = f.p.selection_bits(&duty);
    let unselected = (0..NODE_COUNT)
        .find(|bit| !selected.contains(bit))
        .expect("some node is outside the selection");
    let mut bits = selected[..3].to_vec();
    bits.push(unselected);
    bits.sort_unstable();

    let signed = f.p.sign_duty_with_bits(duty, &bits, None);
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
}

#[test]
fn settlement_rejects_a_quorum_below_signatures_required() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let duty = duty(4);
    let selected = f.p.selection_bits(&duty);

    let signed = f.p.sign_duty_with_bits(duty, &selected[..3], None);
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);
}

// ---------------------------------------------------------------------------
// subscription and gateway state
// ---------------------------------------------------------------------------

#[test]
fn settlement_rejects_an_expired_subscription() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    f.p.env.advance_seconds(Subscription::SECONDS_PER_MONTH + 1);

    let signed = f.p.sign_duty(duty(4));
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::SubscriptionInactiveOrExhausted);
}

#[test]
fn settlement_stops_when_the_round_quota_is_exhausted() {
    let mut f = fixture_with_rounds(2);
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let base = duty(4);

    for offset in 0..2u64 {
        let signed = f.p.sign_duty(base.at(base.canonical_timestamp + offset));
        let ixs = settle_ixs(&f, &owner, None, &signed);
        f.p.env.send_ok(&ixs, &[&owner, &gateway]);
    }

    let signed = f.p.sign_duty(base.at(base.canonical_timestamp + 2));
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::SubscriptionInactiveOrExhausted);

    let subscription: Subscription = f.p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(subscription.used_rounds, 2);
}

#[test]
fn settlement_requires_the_named_gateway_to_sign() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let other_gateway = f.p.register_gateway(0);
    let signed = f.p.sign_duty(duty(4));

    // Build the instruction for `other_gateway`, then point the gateway account at the first one.
    let mut ix = f.p.settle_subscription_round_ix(
        &other_gateway.pubkey(),
        &owner.pubkey(),
        &owner.pubkey(),
        None,
        &signed,
    );
    let other_gateway_pda = pda::gateway(&other_gateway.pubkey());
    let fixture_gateway_pda = pda::gateway(&f.gateway.pubkey());
    let gateway_meta = ix
        .accounts
        .iter_mut()
        .find(|m| m.pubkey == other_gateway_pda)
        .expect("gateway account is present");
    gateway_meta.pubkey = fixture_gateway_pda;
    // Keep round_record seeds aligned with the substituted gateway PDA so
    // `has_one = authority` is what fails, not `ConstraintSeeds`.
    let round_record_meta = ix
        .accounts
        .iter_mut()
        .find(|m| {
            m.pubkey
                == pda::subscription_round(
                    &owner.pubkey(),
                    &other_gateway_pda,
                    &signed.duty.source_id,
                    signed.duty.signatures_required,
                    signed.duty.canonical_timestamp,
                )
        })
        .expect("round_record account is present");
    round_record_meta.pubkey = pda::subscription_round(
        &owner.pubkey(),
        &fixture_gateway_pda,
        &signed.duty.source_id,
        signed.duty.signatures_required,
        signed.duty.canonical_timestamp,
    );

    let ixs = vec![
        f.p.consumer_authorization_ix(&owner, &f.gateway.pubkey(), &signed.duty),
        ix,
    ];
    let failed = f.p.env.send_err(&ixs, &[&owner, &other_gateway]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
}

#[test]
fn settlement_rejects_a_registry_after_its_grace_period() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    // A round signed against the live registry...
    let signed = f.p.sign_duty(duty(4));

    // ...then the registry moves on.
    let authority = f.p.authority.insecure_clone();
    let next = f.p.current_version() + 1;
    let ix = f.p.update_registry_policy_ix(&authority.pubkey(), next, 4);
    f.p.env.send_ok(&[ix], &[&authority]);

    // advance the clock to after the grace period
    f.p.env
        .advance_seconds(f.p.args.max_settlement_delay_seconds as i64 + 1);
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::RegistryNotActive);
}

#[test]
fn settlement_accepts_a_superseded_registry_during_grace() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let signed = f.p.sign_duty(duty(4));

    let authority = f.p.authority.insecure_clone();
    let next = f.p.current_version() + 1;
    let ix = f.p.update_registry_policy_ix(&authority.pubkey(), next, 4);
    f.p.env.send_ok(&[ix], &[&authority]);

    let ixs = settle_ixs(&f, &owner, None, &signed);
    f.p.env.send_ok(&ixs, &[&owner, &gateway]);
}

#[test]
fn settlement_rejects_a_stale_authorized_request() {
    let mut f = fixture();
    let owner = f.owner.insecure_clone();
    let gateway = f.gateway.insecure_clone();
    let now = f.p.env.unix_timestamp() as u64;
    let window = f.p.args.max_settlement_delay_seconds;

    // The consumer authorization matches the duty, so the failure is the freshness window,
    // not the signature: a banked authorization cannot be redeemed after the window expires.
    let stale = duty(4).at(now - window - 1);
    let signed = f.p.sign_duty(stale);
    let ixs = settle_ixs(&f, &owner, None, &signed);
    let failed = f.p.env.send_err(&ixs, &[&owner, &gateway]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);

    // At the boundary the same shape settles.
    let boundary = duty(4).at(now - window);
    let signed = f.p.sign_duty(boundary);
    let ixs = settle_ixs(&f, &owner, None, &signed);
    f.p.env.send_ok(&ixs, &[&owner, &gateway]);
}
