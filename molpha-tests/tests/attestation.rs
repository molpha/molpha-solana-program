//! `submit_attestation`.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use molpha::error::MolphaError;
use molpha::events::FeedUpdated;
use molpha::instructions::SubmitAttestationArgs;
use molpha::state::{Feed, FeedValueKind};
use molpha::utils::MAX_FUTURE_DRIFT_SECONDS;
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation, ZERO_COALITION_KEY};
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
    assert_eq!(feed.value, duty.value);
    assert_eq!(feed.value_kind, FeedValueKind::Value);
    assert_eq!(feed.timestamp, duty.timestamp);
    assert_eq!(feed.signatures_required, SIGS_REQUIRED);
    assert_eq!(feed.registry_version, duty.registry_version);
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);

    let event: FeedUpdated = only_event(&meta);
    assert_eq!(event.source_id, duty.source_id);
    assert_eq!(event.value, duty.value);
    assert_eq!(event.timestamp, duty.timestamp);
    assert_eq!(event.signers_bitmap, signed.signers_bitmap);
    assert_eq!(event.last_updated_slot, p.env.slot());
}

#[test]
fn submit_attestation_overwrites_the_feed_with_a_newer_round() {
    let mut p = with_nodes("submit-newer");
    let submitter = p.env.keypair();
    let first = duty();
    submit(&mut p, &submitter, first);

    let second = first.at(first.timestamp + 60_000).value([0x99; 32]);
    let signed = submit(&mut p, &submitter, second);

    let feed: Feed = p.env.get(&pda::feed(
        &first.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.timestamp, second.timestamp);
    assert_eq!(feed.value, second.value);
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);
}

#[test]
fn submit_attestation_refuses_to_move_the_feed_backwards() {
    let mut p = with_nodes("submit-stale");
    let submitter = p.env.keypair();
    let current = duty().at_secs(1_700_000_500);
    submit(&mut p, &submitter, current);

    for stale_timestamp in [current.timestamp - 1, current.timestamp] {
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
    assert_eq!(feed.timestamp, current.timestamp);
    assert_eq!(feed.value, current.value);
}

/// A stale or duplicate round is refused before its signature is looked at: the monotonicity
/// error wins even over a broken signature, and the refusal costs a small fraction of a full
/// verification.
#[test]
fn a_stale_submit_is_refused_before_the_signature_is_verified() {
    let mut p = with_nodes("submit-stale-cheap");
    let submitter = p.env.keypair();
    let current = duty().at_secs(1_700_000_500);
    submit(&mut p, &submitter, current);

    let stale = p.sign_duty(current.at(current.timestamp - 1).value([0xee; 32]));
    let broken = stale.with_tampered_signature();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &broken);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::FeedNotNewer);

    let fresh = p.sign_duty(current.at(current.timestamp + 1).value([0xdd; 32]));
    let broken_fresh = fresh.with_tampered_signature();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &broken_fresh);
    let verified = p.env.send_err(&[ix], &[&submitter]);
    assert_ne!(
        verified.err.to_string(),
        failed.err.to_string(),
        "a newer round with a broken signature must still reach signature verification"
    );
    assert!(
        failed.meta.compute_units_consumed * 2 < verified.meta.compute_units_consumed,
        "stale refusal used {} CU, full verification {}",
        failed.meta.compute_units_consumed,
        verified.meta.compute_units_consumed
    );
}

/// An attestation still valid under a superseded (grace-period) registry cannot displace a feed
/// already written under the newer one, even with a newer timestamp.
#[test]
fn submit_attestation_refuses_to_move_the_registry_version_backwards() {
    let mut p = with_nodes("submit-version-ratchet");
    let submitter = p.env.keypair();
    let old_version = p.current_version();
    let authority = p.authority.insecure_clone();
    let ix = p.update_registry_policy_ix(&authority.pubkey(), old_version + 1, 4);
    p.env.send_ok(&[ix], &[&authority]);

    let current = Duty::new(old_version + 1, SIGS_REQUIRED).at_secs(1_700_000_500);
    submit(&mut p, &submitter, current);

    let superseded = Duty::new(old_version, SIGS_REQUIRED)
        .at(current.timestamp + 1)
        .value([0xee; 32]);
    let signed = p.sign_duty(superseded);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::RegistryVersionMismatch);

    let feed: Feed = p.env.get(&pda::feed(
        &current.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.registry_version, current.registry_version);
    assert_eq!(feed.value, current.value);
}

// ---------------------------------------------------------------------------
// hashed values
// ---------------------------------------------------------------------------

/// Sign `keccak256(raw)` and submit it with `raw` as the preimage.
fn submit_raw(
    p: &mut Protocol,
    submitter: &Keypair,
    duty: Duty,
    raw: Vec<u8>,
) -> litesvm::types::TransactionResult {
    let signed = p.sign_duty(duty.value(keccak(&raw)));
    submit_with_raw_value(p, submitter, &signed, raw)
}

fn submit_with_raw_value(
    p: &mut Protocol,
    submitter: &Keypair,
    signed: &SignedAttestation,
    raw: Vec<u8>,
) -> litesvm::types::TransactionResult {
    let args = SubmitAttestationArgs {
        attestation: signed.attestation(),
        raw_value: Some(raw),
        coalition_key: signed.coalition_key,
    };
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    p.env.send(&[ix], &[submitter])
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    solana_keccak_hasher::hashv(&[bytes]).to_bytes()
}

#[test]
fn a_raw_value_is_stored_as_its_signed_hash() {
    let mut p = with_nodes("submit-raw-value");
    let submitter = p.env.keypair();
    let raw = vec![0x5a; Feed::MAX_VALUE_LEN];

    let meta = submit_raw(&mut p, &submitter, duty(), raw.clone()).expect("hashed value submits");

    let feed: Feed = p.env.get(&pda::feed(
        &duty().source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.value, keccak(&raw));
    assert_eq!(feed.value_kind, FeedValueKind::Hash);

    let event: FeedUpdated = only_event(&meta);
    assert_eq!(event.value, keccak(&raw));
    assert_eq!(event.value_kind, FeedValueKind::Hash);
}

#[test]
fn a_raw_value_must_hash_to_the_signed_value() {
    let mut p = with_nodes("submit-raw-mismatch");
    let submitter = p.env.keypair();
    // Signed over the plain value, so no preimage can match.
    let signed = p.sign_duty(duty());

    let failed =
        submit_with_raw_value(&mut p, &submitter, &signed, duty().value.to_vec()).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidRawValue);
}

#[test]
fn a_raw_value_longer_than_the_cap_is_rejected() {
    let mut p = with_nodes("submit-raw-too-long");
    let submitter = p.env.keypair();

    let failed = submit_raw(
        &mut p,
        &submitter,
        duty(),
        vec![0x5a; Feed::MAX_VALUE_LEN + 1],
    )
    .unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidRawValue);
}

#[test]
fn each_update_rewrites_the_value_kind() {
    let mut p = with_nodes("submit-kind-flip");
    let submitter = p.env.keypair();
    let feed = pda::feed(&duty().source_id, SIGS_REQUIRED, &submitter.pubkey());

    submit_raw(&mut p, &submitter, duty(), vec![0x01; 64]).expect("hashed value submits");
    assert_eq!(p.env.get::<Feed>(&feed).value_kind, FeedValueKind::Hash);

    let plain = duty().at(duty().timestamp + 1);
    submit(&mut p, &submitter, plain);
    assert_eq!(p.env.get::<Feed>(&feed).value_kind, FeedValueKind::Value);
    assert_eq!(p.env.get::<Feed>(&feed).value, plain.value);
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
    assert_eq!(p.env.get::<Feed>(&a).value, base.value);
    assert_eq!(p.env.get::<Feed>(&b).value, base.value);
    assert_eq!(p.env.get::<Feed>(&c).value, other_source.value);
}

/// `p - y` over the secp256k1 base field, big-endian.
fn field_negate(y: &[u8; 32]) -> [u8; 32] {
    const P: [u8; 32] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0xFF, 0xFF,
        0xFC, 0x2F,
    ];
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let diff = P[i] as i16 - y[i] as i16 - borrow;
        out[i] = diff.rem_euclid(256) as u8;
        borrow = i16::from(diff < 0);
    }
    out
}

#[test]
fn submit_attestation_rejects_a_wrong_coalition_key() {
    let mut p = with_nodes("submit-bad-key");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());
    let key = signed.coalition_key;

    let mut flipped_x = key;
    flipped_x.x[31] ^= 0x01;
    // Same x, other parity: `-key` is on the curve, and only the Y check separates it.
    let mut negated = key;
    negated.y = field_negate(&key.y);
    // A real key of a different signer set: one signer short.
    let signer_keys: Vec<_> = p
        .nodes
        .iter()
        .filter(|n| signed.signer_pdas.contains(&n.pda))
        .map(|n| (n.key.x, n.key.y))
        .collect();
    let subset = molpha_verifier::coalition_key(&signer_keys[1..]).unwrap();

    for bad in [flipped_x, negated, subset, ZERO_COALITION_KEY] {
        let mut args = signed.submit_args();
        args.coalition_key = bad;
        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
        let failed = p.env.send_err(&[ix], &[&submitter]);
        assert_molpha_error(&failed, MolphaError::InvalidCoalitionKey);
    }
    assert!(!p.env.account_exists(&pda::feed(
        &duty().source_id,
        SIGS_REQUIRED,
        &submitter.pubkey()
    )));

    // Any signer order and any library give the same key; the node accounts' sum is accepted.
    let mut reversed = signer_keys.clone();
    reversed.reverse();
    let mut args = signed.submit_args();
    args.coalition_key = molpha_verifier::coalition_key(&reversed).unwrap();
    assert_eq!(
        args.coalition_key,
        p.coalition_key_for(&signed.attestation())
    );
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    p.env.send_ok(&[ix], &[&submitter]);
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

/// Consumers accept any quorum of registered members: selection is a load-balancing device,
/// policed by the unsolicited-signing offence and the activity counters, not by verification.
#[test]
fn submit_attestation_accepts_a_signer_outside_the_derived_selection() {
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
    p.env.send_ok(&[ix], &[&submitter]);

    let feed: Feed = p.env.get(&pda::feed(
        &duty.source_id,
        SIGS_REQUIRED,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);
    assert_eq!(feed.value, duty.value);
}

/// Any registered members at all: the quorum is `k` of the registry, wherever the draw fell.
#[test]
fn submit_attestation_accepts_a_quorum_drawn_entirely_outside_the_selection() {
    let mut p = Protocol::builder().redundancy_buffer(1).start();
    p.register_nodes("submit-all-unselected", 16);
    let submitter = p.env.keypair();
    let duty = Duty::new(16, 5);
    let selected = p.selection_bits(&duty);
    assert_eq!(selected.len(), 6);
    let outside: Vec<usize> = (0..16).filter(|bit| !selected.contains(bit)).collect();
    assert!(outside.len() >= 5);

    let signed = p.sign_duty_with_bits(duty, &outside[..5], None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);
}

#[test]
fn submit_attestation_still_caps_the_coalition_at_quorum_plus_buffer() {
    let mut p = with_nodes("submit-too-many");
    let submitter = p.env.keypair();
    let duty = duty();
    let bits: Vec<usize> = (0..SIGS_REQUIRED as usize + 3).collect();

    let signed = p.sign_duty_with_bits(duty, &bits, None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::CreditedExceedsSelection);
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
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);
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
fn submit_attestation_rejects_a_stale_timestamp() {
    let mut p = with_nodes("submit-too-stale");
    let submitter = p.env.keypair();
    let now = p.env.unix_timestamp() as u64;
    let window = p.args.max_settlement_delay_seconds;

    // A virgin feed (fresh submitter) has no `FeedNotNewer` ratchet, so the window is the
    // only thing stopping a ground timestamp here.
    let stale = duty().at_secs(now - window - 1);
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
    submit(&mut p, &submitter, duty().at_secs(now - window));
}

#[test]
fn submit_attestation_rejects_a_future_timestamp() {
    let mut p = with_nodes("submit-future");
    let submitter = p.env.keypair();
    let now = p.env.unix_timestamp() as u64;

    let future = duty().at_secs(now + MAX_FUTURE_DRIFT_SECONDS + 1);
    let signed = p.sign_duty(future);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationInFuture);

    // Clock drift up to the allowance is accepted.
    submit(
        &mut p,
        &submitter,
        duty().at_secs(now + MAX_FUTURE_DRIFT_SECONDS),
    );
}

/// `timestamp` is unix milliseconds; the clock bounds compare its whole seconds.
#[test]
fn the_clock_bounds_read_the_whole_second_of_the_millisecond_timestamp() {
    let mut p = with_nodes("submit-ms-bounds");
    let now = p.env.unix_timestamp() as u64;
    let window = p.args.max_settlement_delay_seconds;
    let drift_ms = (now + MAX_FUTURE_DRIFT_SECONDS) * 1_000;
    let oldest_ms = (now - window) * 1_000;

    // 999 ms into the last allowed future second is still inside the allowance ...
    let submitter = p.env.keypair();
    submit(&mut p, &submitter, duty().at(drift_ms + 999));
    // ... and the first millisecond of the next second is not.
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty().at(drift_ms + 1_000));
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationInFuture);

    // The oldest accepted second runs to its last millisecond; one millisecond earlier is the
    // second before it.
    let submitter = p.env.keypair();
    submit(&mut p, &submitter, duty().at(oldest_ms + 999));
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty().at(oldest_ms - 1));
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);
}

/// A feed can now update more than once a second: updates order on the full timestamp.
#[test]
fn two_rounds_in_one_second_order_by_milliseconds() {
    let mut p = with_nodes("submit-same-second");
    let submitter = p.env.keypair();
    let second_start = p.env.unix_timestamp() as u64 * 1_000;

    let first = duty().at(second_start + 100);
    submit(&mut p, &submitter, first);
    let later = first.at(second_start + 600).value([0x99; 32]);
    submit(&mut p, &submitter, later);

    let feed_address = pda::feed(&first.source_id, SIGS_REQUIRED, &submitter.pubkey());
    let feed: Feed = p.env.get(&feed_address);
    assert_eq!(feed.value, [0x99; 32]);
    assert_eq!(feed.timestamp, second_start + 600);

    // A round stamped between the two is older than the stored one, even in the same second.
    let between = first.at(second_start + 300).value([0x98; 32]);
    let signed = p.sign_duty(between);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::FeedNotNewer);
    let unchanged: Feed = p.env.get(&feed_address);
    assert_eq!(unchanged.timestamp, second_start + 600);
}

#[test]
fn submit_attestation_update_branch_enforces_freshness() {
    let mut p = with_nodes("submit-stale-update");
    let submitter = p.env.keypair();
    let first = duty().at_secs(p.env.unix_timestamp() as u64);
    submit(&mut p, &submitter, first);

    let window = p.args.max_settlement_delay_seconds;
    p.env.advance_seconds(window as i64 + 100);
    // Newer than the feed, so `FeedNotNewer` would pass — but stale against the clock.
    let stale = first.at(first.timestamp + 1).value([0xee; 32]);
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

    let stale = duty().at_secs(now - 1_001);
    let signed = p.sign_duty(stale);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::AttestationTooStale);

    submit(&mut p, &submitter, duty().at_secs(now - 1_000));
}

/// Signer keys come from the registry snapshot, so a submission carries its fixed accounts and
/// nothing per signer.
#[test]
fn submission_carries_no_per_signer_accounts() {
    let mut p = with_nodes("submit-no-signer-accounts");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());
    assert!(signed.signer_pdas.len() > 1);

    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    assert_eq!(
        ix.accounts.len(),
        5,
        "submitter, registry, feed, config, system"
    );
    p.env.send_ok(&[ix], &[&submitter]);
}

/// A caller still passing signer accounts, as the previous interface had it, is refused rather
/// than ignored.
#[test]
fn a_submission_with_signer_accounts_is_refused() {
    let mut p = with_nodes("submit-stray-signer-accounts");
    let submitter = p.env.keypair();
    let signed = p.sign_duty(duty());
    let mut ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    ix.accounts.push(
        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
            signed.signer_pdas[0],
            false,
        ),
    );
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::UnexpectedSignerAccounts);
}
