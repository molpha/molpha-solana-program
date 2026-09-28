//! Epoch lifecycle: genesis chaining, finalization timing and sequencing, successor creation,
//! the ticket-threshold controller, and recovery after a stall.

use anchor_lang::error::ErrorCode;
use molpha::error::MolphaError;
use molpha::events::{EpochCreated, EpochFinalized};
use molpha::state::{EpochState, TAU_MIN, TAU_STEP};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, events, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{update_config_ix, Protocol};
use solana_signer::Signer;

fn blank_epoch(tau: u64, tickets: u32) -> EpochState {
    let mut state: EpochState = bytemuck::Zeroable::zeroed();
    state.tau = tau;
    state.tickets = tickets;
    state
}

#[test]
fn the_threshold_scales_toward_the_target_one_bounded_step_at_a_time() {
    // On target: unchanged.
    assert_eq!(blank_epoch(1 << 40, 2_000).next_tau(2_000), 1 << 40);
    // Twice the target: halves.
    assert_eq!(blank_epoch(1 << 40, 4_000).next_tau(2_000), 1 << 39);
    // A flood moves it down at most TAU_STEP times.
    assert_eq!(
        blank_epoch(1 << 40, 60_000).next_tau(2_000),
        (1 << 40) / TAU_STEP
    );
    // No tickets counts as one, and the rise is capped at TAU_STEP times.
    assert_eq!(
        blank_epoch(1 << 40, 0).next_tau(2_000),
        (1 << 40) * TAU_STEP
    );
    // Never above sampling every round, never below TAU_MIN, never stuck at zero.
    assert_eq!(blank_epoch(u64::MAX, 0).next_tau(2_000), u64::MAX);
    assert_eq!(blank_epoch(TAU_MIN, 60_000).next_tau(1), TAU_MIN);
    assert_eq!(blank_epoch(0, 0).next_tau(2_000), TAU_MIN);
}

#[test]
fn finalize_waits_for_the_ticket_deadline_and_creates_the_epoch_two_ahead() {
    let mut p = Protocol::start();
    let e0 = p.env.epoch(0);
    let e1 = p.env.epoch(1);
    let payer = p.env.keypair();

    p.env.warp_to(e0.end + e0.ticket_grace - 1);
    let ix = p.finalize_epoch_ix(&payer.pubkey(), 0);
    let failed = p.env.send_err(&[ix], &[&payer]);
    assert_molpha_error(&failed, MolphaError::EpochNotFinalizable);
    assert!(!p.env.account_exists(&pda::epoch(2)));

    p.env.warp_to(e0.end + e0.ticket_grace);
    let ix = p.finalize_epoch_ix(&payer.pubkey(), 0);
    let meta = p.env.send_ok(&[ix], &[&payer]);

    let e0 = p.env.epoch(0);
    assert_eq!(e0.finalized, 1);
    let e2 = p.env.epoch(2);
    assert_eq!(e2.epoch, 2);
    assert_eq!(e2.start, e1.end, "epochs are contiguous");
    assert_eq!(e2.end, e1.end + p.args.epoch_len_seconds as i64);
    assert_eq!(e2.ticket_grace, p.args.ticket_grace_seconds as i64);
    assert_eq!(e2.tau, e0.next_tau(p.args.target_tickets_per_epoch));
    assert_eq!(e2.rent_payer, payer.pubkey().to_bytes());
    assert_eq!(e2.min_samples, p.args.min_availability_samples);

    let finalized: EpochFinalized = only_event(&meta);
    assert_eq!(finalized.epoch, 0);
    assert_eq!(finalized.weight_total, 0);
    let created: EpochCreated = only_event(&meta);
    assert_eq!(
        (created.epoch, created.start, created.end),
        (2, e2.start, e2.end)
    );
}

#[test]
fn an_epoch_is_finalized_once() {
    let mut p = Protocol::start();
    p.finalize_epoch(0);
    let ix = p.finalize_epoch_ix(&p.env.payer_pubkey(), 0);
    let failed = p.env.send_err(&[ix], &[]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|line| line.contains("already in use")));
}

/// `e + 1` exists only once `e - 1` was finalized, so epochs finalize in order.
#[test]
fn epochs_finalize_in_order() {
    let mut p = Protocol::start();
    let e1 = p.env.epoch(1);
    p.env.warp_to(e1.end + e1.ticket_grace);

    // Epoch 2 does not exist yet: nothing has finalized epoch 0.
    let ix = p.finalize_epoch_ix(&p.env.payer_pubkey(), 1);
    let failed = p.env.send_err(&[ix], &[]);
    assert_anchor_error(&failed, ErrorCode::AccountOwnedByWrongProgram);

    p.finalize_epoch(0);
    p.finalize_epoch(1);
    assert!(p.env.account_exists(&pda::epoch(3)));
}

#[test]
fn config_changes_only_reach_epochs_not_yet_created() {
    let mut p = Protocol::start();
    let before = (p.env.epoch(0), p.env.epoch(1));

    let mut args = update_args(&p);
    args.epoch_len_seconds = 43_200;
    args.ticket_grace_seconds = 3_600;
    args.min_availability_samples = 7;
    let authority = p.authority.insecure_clone();
    p.env.send_ok(
        &[update_config_ix(&authority.pubkey(), &args)],
        &[&authority],
    );

    let (e0, e1) = (p.env.epoch(0), p.env.epoch(1));
    assert_eq!(
        (e0.end, e0.ticket_grace),
        (before.0.end, before.0.ticket_grace)
    );
    assert_eq!(
        (e1.end, e1.ticket_grace),
        (before.1.end, before.1.ticket_grace)
    );
    assert_eq!(e1.min_samples, before.1.min_samples);

    p.finalize_epoch(0);
    let e2 = p.env.epoch(2);
    assert_eq!(e2.end - e2.start, 43_200);
    assert_eq!(e2.ticket_grace, 3_600);
    assert_eq!(e2.min_samples, 7);
}

/// A new epoch must outlast its predecessor's ticket grace. Otherwise the predecessor could only
/// be finalized after the new epoch ended, and the epoch after it would open late.
#[test]
fn a_shorter_epoch_length_never_leaves_a_gap_without_an_open_epoch() {
    let mut p = Protocol::start();
    let mut args = update_args(&p);
    args.epoch_len_seconds = molpha::state::MIN_EPOCH_LEN_SECONDS;
    args.ticket_grace_seconds = 1_800;
    let authority = p.authority.insecure_clone();
    p.env.send_ok(
        &[update_config_ix(&authority.pubkey(), &args)],
        &[&authority],
    );

    p.finalize_epoch(0);
    let e1 = p.env.epoch(1);
    let e2 = p.env.epoch(2);
    assert_eq!(e2.end - e2.start, e1.ticket_grace + 1);

    // e1 finalizes (creating e3) while e2 is still open, so e3 exists before it starts.
    p.finalize_epoch(1);
    assert!(p.env.unix_timestamp() < e2.end);
    let e3 = p.env.epoch(3);
    assert_eq!(e3.start, e2.end);
    assert_eq!(
        e3.end - e3.start,
        3_600,
        "the new length applies once it is safe"
    );
}

/// If nobody finalizes for several epochs, sweeps stall (no open epoch account) until anyone
/// catches the chain up, one finalize per epoch.
#[test]
fn a_stalled_chain_catches_up_permissionlessly() {
    let mut p = Protocol::start();
    p.register_nodes("stall", 4);
    let len = p.args.epoch_len_seconds as i64;
    let start = p.env.epoch(0).start;
    p.env.warp_to(start + 5 * len + 10);
    p.fund_treasury(1_000);

    // Epoch 1 is the newest account and it is long over.
    let ix = p.sweep_treasury_revenue_ix(1);
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotOpen);

    let payer = p.env.payer_pubkey();
    let mut created = Vec::new();
    for epoch in 0..4 {
        let meta = p.env.send_ok(&[p.finalize_epoch_ix(&payer, epoch)], &[]);
        created.extend(events::<EpochCreated>(&meta).into_iter().map(|e| e.epoch));
    }
    assert_eq!(created, vec![2, 3, 4, 5]);
    let open = p.open_epoch();
    assert_eq!(open, 5);
    p.env.send_ok(&[p.sweep_treasury_revenue_ix(open)], &[]);
    assert!(p.env.epoch(open).pool > 0);
    p.assert_solvent();
}

fn update_args(p: &Protocol) -> molpha::instructions::UpdateConfigArgs {
    let a = &p.args;
    molpha::instructions::UpdateConfigArgs {
        withdrawal_cooldown_slots: a.withdrawal_cooldown_slots,
        minimum_node_deposit: a.minimum_node_deposit,
        reward_per_signature: a.reward_per_signature,
        max_settlement_delay_seconds: a.max_settlement_delay_seconds,
        challenger_bounty_bps: a.challenger_bounty_bps,
        x402_round_base: a.x402_round_base,
        min_signers: a.min_signers,
        epoch_len_seconds: a.epoch_len_seconds,
        ticket_grace_seconds: a.ticket_grace_seconds,
        target_tickets_per_epoch: a.target_tickets_per_epoch,
        min_availability_bps: a.min_availability_bps,
        min_availability_samples: a.min_availability_samples,
        protocol_fee_bps: a.protocol_fee_bps,
    }
}
