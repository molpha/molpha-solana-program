//! `submit_attestation`.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::FeedUpdated;
use molpha::state::{Feed, FeedValueKind};
use molpha::utils::MAX_FUTURE_DRIFT_SECONDS;
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event, readonly_metas};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS_REQUIRED: u8 = 5;

/// A protocol with `NODE_COUNT` registered nodes; the live registry version equals the count.
fn with_nodes(domain: &str) -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes(domain, NODE_COUNT);
    p
}

fn duty() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS_REQUIRED)
}

/// Sign `duty` with the full selection set and submit it.
fn submit(p: &mut Protocol, submitter: &Keypair, duty: Duty) -> SignedAttestation {
    let signed = p.sign_duty(duty);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[submitter]);
    signed
}

#[test]
fn submit_attestation_creates_the_feed_and_records_the_round() {
    let mut p = with_nodes("submit-create");
    let submitter = p.env.keypair();
    let duty = duty();
    let signed = p.sign_duty(duty);

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let meta = p.env.send_ok(&[ix], &[&submitter]);

    let feed: Feed = p.env.get(&pda::feed(
        &duty.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.source_id, duty.source_id);
    assert_eq!(feed.value, duty.value.to_vec());
    assert_eq!(feed.value_kind, FeedValueKind::Value);
    assert_eq!(feed.canonical_timestamp, duty.canonical_timestamp);
    assert_eq!(feed.signatures_required, SIGS_REQUIRED);
    assert_eq!(feed.registry_version, duty.registry_version);
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);

    let event: FeedUpdated = only_event(&meta);
    assert_eq!(event.source_id, duty.source_id);
    assert_eq!(event.value, duty.value);
    assert_eq!(event.canonical_timestamp, duty.canonical_timestamp);
    assert_eq!(event.signers_bitmap, signed.signers_bitmap);
    assert_eq!(event.last_updated_slot, p.env.slot());
}

#[test]
fn submit_attestation_overwrites_the_feed_with_a_newer_round() {
    let mut p = with_nodes("submit-newer");
    let submitter = p.env.keypair();
    let first = duty();
    submit(&mut p, &submitter, first);

    let second = first.at(first.canonical_timestamp + 60).value([0x99; 32]);
    let signed = submit(&mut p, &submitter, second);

    let feed: Feed = p.env.get(&pda::feed(
        &first.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.canonical_timestamp, second.canonical_timestamp);
    assert_eq!(feed.value, second.value.to_vec());
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);
}

#[test]
fn submit_attestation_refuses_to_move_the_feed_backwards() {
    let mut p = with_nodes("submit-stale");
    let submitter = p.env.keypair();
    let current = duty().at(1_700_000_500);
    submit(&mut p, &submitter, current);

    for stale_timestamp in [current.canonical_timestamp - 1, current.canonical_timestamp] {
        let stale = current.at(stale_timestamp).value([0xee; 32]);
        let signed = p.sign_duty(stale);
        let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
        let failed = p.env.send_err(&[ix], &[&submitter]);
        assert_molpha_error(&failed, MolphaError::FeedNotNewer);
    }

    let feed: Feed = p.env.get(&pda::feed(
        &current.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.canonical_timestamp, current.canonical_timestamp);
    assert_eq!(feed.value, current.value.to_vec());
}

#[test]
fn feeds_are_scoped_to_submitter_source_and_quorum() {
    let mut p = with_nodes("submit-scoping");
    let first = p.env.keypair();
    let second = p.env.keypair();
    let base = duty();

    submit(&mut p, &first, base);
    // Same duty, different submitter -> a separate feed account.
    submit(&mut p, &second, base);
    // Same submitter, different source -> a separate feed account.
    let other_source = base.source_id([0x77; 32]).value([0x01; 32]);
    submit(&mut p, &first, other_source);

    let a = pda::feed(&base.source_id, SIGS_REQUIRED, &first.pubkey());
    let b = pda::feed(&base.source_id, SIGS_REQUIRED, &second.pubkey());
    let c = pda::feed(&other_source.source_id, SIGS_REQUIRED, &first.pubkey());
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(p.env.get::<Feed>(&a).value, base.value.to_vec());
    assert_eq!(p.env.get::<Feed>(&b).value, base.value.to_vec());
    assert_eq!(p.env.get::<Feed>(&c).value, other_source.value.to_vec());
}

#[test]
fn submit_attestation_rejects_a_tampered_aggregate_signature() {
    let mut p = with_nodes("submit-tampered");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty()).with_tampered_signature();

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    assert!(!p.env.account_exists(&pda::feed(
        &duty().source_id,
        SIGS_REQUIRED,
        &submitter.pubkey()
    )));
}

#[test]
fn submit_attestation_rejects_a_value_the_coalition_did_not_sign() {
    let mut p = with_nodes("submit-swapped-value");
    let submitter = p.env.keypair();
    let mut signed = p.sign_duty(duty());
    // The signature commits to the value; swapping it after the fact must not verify.
    signed.duty.value = [0xab; 32];

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
}

#[test]
fn submit_attestation_rejects_a_signer_outside_the_derived_selection() {
    let mut p = with_nodes("submit-unselected");
    let submitter = p.env.keypair();
    let duty = duty();

    let selected = p.selection_bits(&duty);
    let unselected = (0..NODE_COUNT)
        .find(|bit| !selected.contains(bit))
        .expect("redundancy_buffer 2 over 8 nodes leaves one node out");
    let mut bits = selected[..SIGS_REQUIRED as usize - 1].to_vec();
    bits.push(unselected);
    bits.sort_unstable();

    let signed = p.sign_duty_with_bits(duty, &bits, None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
}

#[test]
fn submit_attestation_rejects_a_quorum_below_signatures_required() {
    let mut p = with_nodes("submit-quorum");
    let submitter = p.env.keypair();
    let duty = duty();
    let selected = p.selection_bits(&duty);

    let signed = p.sign_duty_with_bits(duty, &selected[..SIGS_REQUIRED as usize - 1], None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InsufficientSigners);
}

#[test]
fn submit_attestation_accepts_the_full_selection_set() {
    let mut p = with_nodes("submit-full-selection");
    let submitter = p.env.keypair();
    let duty = duty();
    let selected = p.selection_bits(&duty);
    assert_eq!(
        selected.len(),
        SIGS_REQUIRED as usize + p.args.redundancy_buffer as usize
    );

    // The whole selection may sign, not just `signatures_required` of it.
    let signed = p.sign_duty_with_bits(duty, &selected, None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);
    let feed: Feed = p.env.get(&pda::feed(
        &duty.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.signers_bitmap, crypto::bitmap_from_bits(&selected));
}

#[test]
fn submit_attestation_rejects_signer_accounts_in_the_wrong_order() {
    let mut p = with_nodes("submit-order");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());

    let mut ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let n = ix.accounts.len();
    ix.accounts.swap(n - 1, n - 2);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::MissingSignerAccount);
}

#[test]
fn submit_attestation_rejects_a_missing_signer_account() {
    let mut p = with_nodes("submit-missing");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());

    let mut ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    ix.accounts.pop();
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::MissingSignerAccount);
}

#[test]
fn submit_attestation_rejects_a_signer_account_from_outside_the_program() {
    let mut p = with_nodes("submit-foreign");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());

    let mut ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let last = ix.accounts.len() - 1;
    // A system-owned account cannot masquerade as a Node.
    ix.accounts[last] = readonly_metas([Pubkey::new_unique()]).remove(0);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::MissingSignerAccount);
}

#[test]
fn submit_attestation_rejects_a_registry_account_that_is_not_the_named_version() {
    let mut p = with_nodes("submit-registry-pda");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());

    let mut ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    // Account index 1 is `registry` (0 is `submitter`).
    ix.accounts[1].pubkey = pda::registry(duty().registry_version - 1);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

// ---------------------------------------------------------------------------
// freshness window
// ---------------------------------------------------------------------------

#[test]
fn submit_attestation_rejects_a_stale_canonical_timestamp() {
    let mut p = with_nodes("submit-too-stale");
    let submitter = p.env.keypair();
    let now = p.env.unix_timestamp() as u64;
    let window = p.args.max_settlement_delay_seconds;

    // A virgin feed (fresh submitter) has no `FeedNotNewer` ratchet, so the window is the
    // only thing stopping a ground timestamp here.
    let stale = duty().at(now - window - 1);
    let signed = p.sign_duty(stale);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);
    assert!(!p.env.account_exists(&pda::feed(
        &stale.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey()
    )));

    // The window is inclusive.
    submit(&mut p, &submitter, duty().at(now - window));
}

#[test]
fn submit_attestation_rejects_a_future_canonical_timestamp() {
    let mut p = with_nodes("submit-future");
    let submitter = p.env.keypair();
    let now = p.env.unix_timestamp() as u64;

    let future = duty().at(now + MAX_FUTURE_DRIFT_SECONDS + 1);
    let signed = p.sign_duty(future);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationInFuture);

    // Clock drift up to the allowance is accepted.
    submit(
        &mut p,
        &submitter,
        duty().at(now + MAX_FUTURE_DRIFT_SECONDS),
    );
}

#[test]
fn submit_attestation_update_branch_enforces_freshness() {
    let mut p = with_nodes("submit-stale-update");
    let submitter = p.env.keypair();
    let first = duty().at(p.env.unix_timestamp() as u64);
    submit(&mut p, &submitter, first);

    let window = p.args.max_settlement_delay_seconds;
    p.env.advance_seconds(window as i64 + 100);
    // Newer than the feed, so `FeedNotNewer` would pass — but stale against the clock.
    let stale = first.at(first.canonical_timestamp + 1).value([0xee; 32]);
    let signed = p.sign_duty(stale);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);
}

#[test]
fn the_freshness_window_comes_from_config() {
    let mut p = Protocol::builder()
        .max_settlement_delay_seconds(1_000)
        .start();
    p.register_nodes("submit-window", NODE_COUNT);
    let submitter = p.env.keypair();
    let now = p.env.unix_timestamp() as u64;

    let stale = duty().at(now - 1_001);
    let signed = p.sign_duty(stale);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);

    submit(&mut p, &submitter, duty().at(now - 1_000));
}
