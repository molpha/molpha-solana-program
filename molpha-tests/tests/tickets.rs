//! `submit_ticket` and `close_ticket_marker`: sampled rounds as verified participation evidence.

use anchor_lang::error::ErrorCode;
use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::TicketSubmitted;
use molpha::state::{ticket_id, TicketMarker, MAX_TICKETS_PER_EPOCH};
use molpha_tests::env::{
    assert_anchor_error, assert_molpha_error, only_event, set_compute_unit_price,
};
use molpha_tests::pda;
use molpha_tests::scenario::{update_config_ix, Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;

/// Eight nodes, quorum 3 plus buffer 2: five selected per round, three sign.
fn setup() -> (Protocol, Keypair, Keypair) {
    let mut p = Protocol::start();
    p.register_nodes("tickets", NODE_COUNT);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    (p, gateway, submitter)
}

fn roster_keys(p: &Protocol, epoch: u64) -> Vec<Pubkey> {
    let state = p.env.epoch(epoch);
    state.roster[..usize::from(state.roster_len)]
        .iter()
        .map(|key| Pubkey::new_from_array(*key))
        .collect()
}

// The failure metadata carries the logs, which is what makes a failing test readable.
#[allow(clippy::result_large_err)]
fn send(
    p: &mut Protocol,
    submitter: &Keypair,
    gateway: &Keypair,
    signed: &SignedAttestation,
) -> litesvm::types::TransactionResult {
    let ixs = p.ticket_ixs(&submitter.pubkey(), gateway, signed);
    p.env.send(&ixs, &[submitter])
}

/// A ticket names no signer accounts: one passed anyway, as the previous interface had it, is
/// refused rather than ignored.
#[test]
fn a_ticket_with_signer_accounts_is_refused() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let mut ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let ticket = ixs.last_mut().unwrap();
    ticket.accounts.push(
        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
            signed.signer_pdas[0],
            false,
        ),
    );
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::UnexpectedSignerAccounts);
    assert_eq!(p.env.epoch(0).tickets, 0);
}

#[test]
fn a_ticket_counts_its_whole_selection_and_records_a_marker() {
    let (mut p, gateway, submitter) = setup();
    let duty = p.live_duty(3);
    let signed = p.sign_canonical(duty);
    let selection: Vec<Pubkey> = {
        let registry = p.env.registry(duty.registry_version);
        p.selection_bits(&duty)
            .into_iter()
            .map(|bit| registry.nodes[bit])
            .collect()
    };
    assert_eq!(selection.len(), 5);

    let meta = p.submit_ticket(&submitter, &gateway, &signed);

    let state = p.env.epoch(0);
    assert_eq!(state.tickets, 1);
    assert_eq!(
        roster_keys(&p, 0),
        selection,
        "roster fills in selection order"
    );
    for (slot, node) in selection.iter().enumerate() {
        assert_eq!(state.selected[slot], 1);
        let signed_now = u16::from(signed.signer_pdas.contains(node));
        assert_eq!(state.signed[slot], signed_now);
    }
    assert_eq!((state.sum_signed, state.sum_selected), (3, 5));

    let marker_key = pda::ticket_marker(&ticket_id(&duty.payload()));
    let marker: TicketMarker = p.env.get(&marker_key);
    assert_eq!(marker.submitter, submitter.pubkey());
    assert_eq!(marker.closable_after, state.end + state.ticket_grace);

    let event: TicketSubmitted = only_event(&meta);
    assert_eq!(event.epoch, 0);
    assert_eq!(event.gateway, pda::gateway(&gateway.pubkey()));
    assert_eq!(event.submitter, submitter.pubkey());
    assert_eq!(event.signers_bitmap, signed.signers_bitmap);
}

/// Round identity keys the marker, so another signer subset, another value, or a re-signed
/// aggregate of the same round can never count twice.
#[test]
fn a_round_counts_once_whatever_aggregate_carries_it() {
    let (mut p, gateway, submitter) = setup();
    let duty = p.live_duty(3);
    p.submit_ticket(&submitter, &gateway, &p.sign_canonical(duty));

    let bits = p.selection_bits(&duty);
    let other_subset = p.sign_duty_with_bits(duty, &bits[1..4], None);
    let failed = send(&mut p, &submitter, &gateway, &other_subset).unwrap_err();
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));

    let resigned = p.sign_duty_with_bits(duty.value([0x99; 32]), &bits[..3], Some([7; 32]));
    let failed = send(&mut p, &submitter, &gateway, &resigned).unwrap_err();
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));
    assert_eq!(p.env.epoch(0).tickets, 1);
}

#[test]
fn a_round_belongs_to_the_epoch_its_timestamp_falls_in() {
    let (mut p, gateway, submitter) = setup();
    let e0 = p.env.epoch(0);

    // Before genesis.
    let early = p.sign_canonical(p.live_duty(3).at_secs(e0.start as u64 - 1));
    let ixs = vec![
        p.dispatch_ix(&gateway, &early.duty),
        p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &early),
    ];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::RoundOutsideEpoch);

    // The epoch window is in seconds: the last millisecond before genesis is still outside it.
    let edge = p.sign_canonical(p.live_duty(3).at(e0.start as u64 * 1_000 - 1));
    let ixs = vec![
        p.dispatch_ix(&gateway, &edge.duty),
        p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &edge),
    ];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::RoundOutsideEpoch);

    // A round of epoch 0 filed against epoch 1.
    let round = p.sign_canonical(p.live_duty(3));
    let ixs = vec![
        p.dispatch_ix(&gateway, &round.duty),
        p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 1, &round),
    ];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::RoundOutsideEpoch);
}

#[test]
fn late_tickets_are_accepted_until_the_ticket_deadline() {
    let (mut p, gateway, submitter) = setup();
    let e0 = p.env.epoch(0);
    let late = p.sign_canonical(p.live_duty(3).source_id([1; 32]));
    let too_late = p.sign_canonical(p.live_duty(3).source_id([2; 32]));

    p.env.warp_to(e0.end + e0.ticket_grace - 1);
    p.submit_ticket(&submitter, &gateway, &late);

    p.env.warp_to(e0.end + e0.ticket_grace);
    let failed = send(&mut p, &submitter, &gateway, &too_late).unwrap_err();
    assert_molpha_error(&failed, MolphaError::TicketWindowClosed);
    assert_eq!(p.env.epoch(0).tickets, 1);
}

#[test]
fn rounds_from_the_future_are_rejected() {
    let (mut p, gateway, submitter) = setup();
    let now = p.env.unix_timestamp() as u64;
    let at_drift = p.sign_canonical(p.live_duty(3).at_secs(now + 60));
    p.submit_ticket(&submitter, &gateway, &at_drift);

    let beyond = p.sign_canonical(p.live_duty(3).at_secs(now + 61));
    let failed = send(&mut p, &submitter, &gateway, &beyond).unwrap_err();
    assert_molpha_error(&failed, MolphaError::AttestationInFuture);
}

/// Tickets arrive long after their round, so the registry version must have been live when the
/// round ran, not when the ticket lands.
#[test]
fn the_registry_version_must_have_been_live_at_the_round() {
    let mut p = Protocol::builder().max_settlement_delay_seconds(60).start();
    p.register_nodes("ticket-registry", NODE_COUNT);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let version = p.current_version();

    // A round signed while `version` was current stays valid after it is superseded and its
    // grace has run out.
    let before = p.sign_canonical(p.live_duty(3));
    p.register_nodes("ticket-registry-late", 1);
    p.env.advance_seconds(120);
    p.submit_ticket(&submitter, &gateway, &before);

    // A round claiming `version` after its grace expired is not.
    let after = p.sign_canonical(p.live_duty(3).source_id([0x44; 32]));
    assert_eq!(after.duty.registry_version, version + 1);
    let stale = p.sign_canonical(Duty::new(version, 3).at_secs(p.env.unix_timestamp() as u64));
    let failed = send(&mut p, &submitter, &gateway, &stale).unwrap_err();
    assert_molpha_error(&failed, MolphaError::RegistryNotActive);
}

#[test]
fn a_registry_cannot_count_a_round_dated_before_its_creation() {
    let mut p = Protocol::start();
    let timestamp = p.env.unix_timestamp() as u64;
    p.env.advance_seconds(120);
    p.register_nodes("ticket-backdate", NODE_COUNT);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let signed = p.sign_canonical(p.live_duty(3).at_secs(timestamp));
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::RegistryNotActive);
    assert_eq!(p.env.epoch(0).tickets, 0);
}

#[test]
fn a_quorum_floor_update_does_not_invalidate_an_existing_epochs_tickets() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let a = &p.args;
    let args = molpha::instructions::UpdateConfigArgs {
        withdrawal_cooldown_slots: a.withdrawal_cooldown_slots,
        minimum_node_deposit: a.minimum_node_deposit,
        reward_per_signature: a.reward_per_signature,
        max_settlement_delay_seconds: a.max_settlement_delay_seconds,
        challenger_bounty_bps: a.challenger_bounty_bps,
        x402_round_base: a.x402_round_base,
        min_signers: 4,
        epoch_len_seconds: a.epoch_len_seconds,
        ticket_grace_seconds: a.ticket_grace_seconds,
        target_tickets_per_epoch: a.target_tickets_per_epoch,
        min_availability_bps: a.min_availability_bps,
        min_availability_samples: a.min_availability_samples,
        protocol_fee_bps: a.protocol_fee_bps,
    };
    let authority = p.authority.insecure_clone();
    p.env.send_ok(
        &[update_config_ix(&authority.pubkey(), &args)],
        &[&authority],
    );
    p.submit_ticket(&submitter, &gateway, &signed);
    assert_eq!(p.env.epoch(0).tickets, 1);
    assert_eq!(p.env.epoch(0).min_signers, 3);
    p.finalize_epoch(0);
    assert_eq!(p.env.epoch(2).min_signers, 4);
}

#[test]
fn a_quorum_below_the_protocol_floor_is_rejected() {
    let (mut p, gateway, submitter) = setup();
    let duty = p.live_duty(2);
    let bits = p.selection_bits(&duty);
    let signed = p.sign_duty_with_bits(duty, &bits[..2], None);
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::SignaturesBelowMinSigners);
}

#[test]
fn a_round_above_the_ticket_threshold_is_not_a_ticket() {
    let (mut p, gateway, submitter) = setup();
    p.env.overwrite_epoch(0, |epoch| epoch.tau = 0);
    let signed = p.sign_canonical(p.live_duty(3));
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::NotATicket);
}

#[test]
fn an_invalid_aggregate_is_rejected() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3)).with_tampered_signature();
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    assert_eq!(p.env.epoch(0).tickets, 0);
}

#[test]
fn a_ticket_needs_the_gateway_dispatch_signature() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let ticket = p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed);

    // Missing.
    let failed = p.env.send_err(std::slice::from_ref(&ticket), &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);

    // Signed by someone other than the named gateway.
    let impostor = p.env.keypair();
    let ixs = vec![p.dispatch_ix(&impostor, &signed.duty), ticket.clone()];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);

    // Over another round.
    let other = signed.duty.at(signed.duty.timestamp - 1);
    let ixs = vec![p.dispatch_ix(&gateway, &other), ticket.clone()];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);

    // Valid, but not immediately before the ticket.
    let ixs = vec![
        p.dispatch_ix(&gateway, &signed.duty),
        set_compute_unit_price(1),
        ticket.clone(),
    ];
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);

    p.env.send_ok(
        &[p.dispatch_ix(&gateway, &signed.duty), ticket],
        &[&submitter],
    );
}

#[test]
fn an_unregistered_gateway_cannot_dispatch() {
    let (mut p, _gateway, submitter) = setup();
    let stranger = p.env.keypair();
    let signed = p.sign_canonical(p.live_duty(3));
    let ixs = p.ticket_ixs(&submitter.pubkey(), &stranger, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_anchor_error(&failed, ErrorCode::AccountNotInitialized);
}

#[test]
fn roster_hints_must_name_the_selected_nodes_exactly() {
    let (mut p, gateway, submitter) = setup();
    p.submit_ticket(&submitter, &gateway, &p.sign_canonical(p.live_duty(3)));
    let roster_len = p.env.epoch(0).roster_len;

    let signed = p.sign_canonical(p.live_duty(3).source_id([0x21; 32]));
    let good = p.roster_slots(0, &signed.duty);
    let known = good
        .iter()
        .position(|slot| *slot < roster_len)
        .expect("a selected node is already on the roster");

    let mut cases: Vec<Vec<u16>> = Vec::new();
    // Wrong length.
    cases.push(good[..good.len() - 1].to_vec());
    // Existing slot of a different node.
    let mut swapped = good.clone();
    swapped[known] = (good[known] + 1) % roster_len;
    cases.push(swapped);
    // Appending an already-rostered node would leave it uncounted.
    let mut duplicate = good.clone();
    duplicate[known] = roster_len;
    cases.push(duplicate);
    // A gap past the roster end.
    let mut gap = good.clone();
    gap[known] = roster_len + 3;
    cases.push(gap);

    for hints in cases {
        let ixs = vec![
            p.dispatch_ix(&gateway, &signed.duty),
            p.submit_ticket_raw_ix(
                &submitter.pubkey(),
                &gateway.pubkey(),
                0,
                signed.attestation(),
                hints,
                signed.coalition_key,
            ),
        ];
        let failed = p.env.send_err(&ixs, &[&submitter]);
        assert_molpha_error(&failed, MolphaError::InvalidRosterSlot);
    }
    p.submit_ticket(&submitter, &gateway, &signed);
}

/// Registry positions move on swap-remove; counters follow the node, not the position.
#[test]
fn counters_follow_nodes_across_registry_versions() {
    let (mut p, gateway, submitter) = setup();
    let first = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &first);

    // Deactivating node 0 moves the last node into its registry position.
    let owner = p.nodes[0].owner.insecure_clone();
    p.env
        .send_ok(&[p.deactivate_node_ix(0, &owner.pubkey())], &[&owner]);
    let second = p.sign_canonical(p.live_duty(3).source_id([0x33; 32]));
    assert_eq!(
        second.duty.registry_version,
        first.duty.registry_version + 1
    );
    p.submit_ticket(&submitter, &gateway, &second);

    let state = p.env.epoch(0);
    for node in p.node_pdas() {
        let expected_signed = [&first, &second]
            .iter()
            .filter(|signed| signed.signer_pdas.contains(&node))
            .count() as u16;
        let slot = p.roster_slot_of(0, &node);
        let counted = slot.map_or(0, |slot| state.signed[usize::from(slot)]);
        assert_eq!(counted, expected_signed, "signed count for {node}");
    }
    assert_eq!(state.sum_signed, 6);
}

#[test]
fn the_ticket_cap_is_enforced() {
    let (mut p, gateway, submitter) = setup();
    p.env
        .overwrite_epoch(0, |epoch| epoch.tickets = MAX_TICKETS_PER_EPOCH);
    let signed = p.sign_canonical(p.live_duty(3));
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::TicketCapReached);
}

#[test]
fn a_finalized_epoch_accepts_no_tickets() {
    let (mut p, gateway, submitter) = setup();
    p.env.overwrite_epoch(0, |epoch| epoch.finalized = 1);
    let signed = p.sign_canonical(p.live_duty(3));
    let failed = send(&mut p, &submitter, &gateway, &signed).unwrap_err();
    assert_molpha_error(&failed, MolphaError::TicketWindowClosed);
}

#[test]
fn a_marker_returns_its_rent_once_the_round_can_no_longer_be_submitted() {
    let (mut p, gateway, submitter) = setup();
    let duty = p.live_duty(3);
    p.submit_ticket(&submitter, &gateway, &p.sign_canonical(duty));
    let marker = pda::ticket_marker(&ticket_id(&duty.payload()));
    let closable_after = p.env.get::<TicketMarker>(&marker).closable_after;
    let crank = p.env.keypair();

    p.env.warp_to(closable_after - 1);
    let ix = p.close_ticket_marker_ix(&marker, &submitter.pubkey());
    let failed = p.env.send_err(std::slice::from_ref(&ix), &[&crank]);
    assert_molpha_error(&failed, MolphaError::TicketMarkerNotExpired);

    p.env.warp_to(closable_after);
    let wrong = p.close_ticket_marker_ix(&marker, &crank.pubkey());
    let failed = p.env.send_err(&[wrong], &[&crank]);
    assert_molpha_error(&failed, MolphaError::InvalidRentReceiver);

    let rent = p.env.lamports(&marker);
    let before = p.env.lamports(&submitter.pubkey());
    p.env.send_ok(&[ix], &[&crank]);
    assert!(!p.env.account_exists(&marker));
    assert_eq!(p.env.lamports(&submitter.pubkey()), before + rent);
}
