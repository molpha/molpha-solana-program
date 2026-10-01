//! Payout: finalized weights, `credit_epoch_rewards`, forfeits, `close_epoch` and claims.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{EpochClosed, EpochRewardCredited, RewardsClaimed};
use molpha::state::{Node, NodeStatus};
use molpha_tests::env::{assert_molpha_error, events, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_keypair::Keypair;
use solana_signer::Signer;

const INCOME: u64 = 10_000_000;

/// Four nodes, quorum 3 plus buffer 2: every round selects all four and three sign.
fn setup(min_samples: u16) -> (Protocol, Keypair) {
    let mut p = Protocol::builder()
        .min_availability_samples(min_samples)
        .start();
    p.register_nodes("epoch-rewards", 4);
    let gateway = p.register_gateway();
    (p, gateway)
}

fn claimable(p: &Protocol, node: &Pubkey) -> u64 {
    p.env.get::<Node>(node).claimable_rewards
}

/// Submit one ticket per signer set, each on its own source.
fn submit_rounds(p: &mut Protocol, gateway: &Keypair, excluded_per_round: &[Pubkey]) {
    let submitter = p.env.keypair();
    for (round, excluded) in excluded_per_round.iter().enumerate() {
        let duty = p.live_duty(3).source_id([round as u8 + 1; 32]);
        let signed = p.sign_excluding(duty, &[*excluded]);
        p.submit_ticket(&submitter, gateway, &signed);
    }
}

#[test]
fn a_full_cycle_pays_each_node_its_share_of_income() {
    let (mut p, gateway) = setup(50);
    let epoch = p.run_reward_epoch(&gateway, INCOME, 5);
    let state = p.env.epoch(epoch);
    assert_eq!(state.finalized, 1);
    assert_eq!(state.tickets, 5);

    let mut paid = 0;
    for node in p.node_pdas() {
        let slot = usize::from(p.roster_slot_of(epoch, &node).unwrap());
        let expected = state.pool * state.weight(slot) / state.weight_total;
        assert_eq!(claimable(&p, &node), expected);
        paid += expected;
    }
    assert_eq!(state.paid, paid);
    assert!(
        paid <= state.pool && state.pool - paid < 4,
        "only rounding dust is left"
    );
    assert_eq!(p.config().pool_liability, state.pool - paid);
    assert_eq!(p.reward_liability(), paid);
    p.assert_solvent();

    // The existing claim path withdraws the credit from the treasury.
    let owner = p.nodes[0].owner.insecure_clone();
    let credit = claimable(&p, &p.nodes[0].pda);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance_or_zero(&destination);
    let meta = p
        .env
        .send_ok(&[p.claim_node_rewards_ix(&owner.pubkey())], &[&owner]);
    assert_eq!(p.env.token_balance(&destination), before + credit);
    assert_eq!(only_event::<RewardsClaimed>(&meta).amount, credit);
    p.assert_solvent();
}

/// Shares follow sampled signatures: a node in twice as many signer sets earns twice as much.
#[test]
fn shares_follow_sampled_signature_counts() {
    let (mut p, gateway) = setup(50);
    let [a, b, c, d] = [
        p.nodes[0].pda,
        p.nodes[1].pda,
        p.nodes[2].pda,
        p.nodes[3].pda,
    ];
    p.earn_income(INCOME);
    // d misses two rounds, c misses one, a and b never miss.
    submit_rounds(&mut p, &gateway, &[d, d, c]);
    p.finalize_epoch(0);
    p.credit_all(0);

    let state = p.env.epoch(0);
    let signed = |node: &Pubkey| state.signed[usize::from(p.roster_slot_of(0, node).unwrap())];
    assert_eq!(
        (signed(&a), signed(&b), signed(&c), signed(&d)),
        (3, 3, 2, 1)
    );
    assert_eq!(state.weight_total, 9);
    for (node, weight) in [(a, 3), (b, 3), (c, 2), (d, 1)] {
        assert_eq!(claimable(&p, &node), state.pool * weight / 9);
    }
    p.assert_solvent();
}

/// Below the relative availability floor a node earns nothing, and its share goes to the nodes
/// that did the work.
#[test]
fn the_availability_floor_zeroes_a_mostly_offline_node() {
    let (mut p, gateway) = setup(4);
    let [a, b, c, d] = [
        p.nodes[0].pda,
        p.nodes[1].pda,
        p.nodes[2].pda,
        p.nodes[3].pda,
    ];
    p.earn_income(INCOME);
    // d signs one round in four: 25% inclusion against a network rate of 75%.
    submit_rounds(&mut p, &gateway, &[d, d, d, a]);
    p.finalize_epoch(0);

    let state = p.env.epoch(0);
    let d_slot = usize::from(p.roster_slot_of(0, &d).unwrap());
    assert_eq!(state.signed[d_slot], 1);
    assert_eq!(state.weight(d_slot), 0);
    assert_eq!(state.weighted_count, 3);
    assert_eq!(state.weight_total, 3 + 4 + 4);

    p.credit_all(0);
    assert_eq!(claimable(&p, &d), 0);
    let paid: u64 = [a, b, c].iter().map(|node| claimable(&p, node)).sum();
    assert!(state.pool - paid < 3);
    p.assert_solvent();
}

/// Below `min_samples` the floor never fires: a node sampled only a few times keeps its share.
#[test]
fn the_sample_minimum_keeps_a_lightly_sampled_node_paid() {
    let (mut p, gateway) = setup(5);
    let [a, d] = [p.nodes[0].pda, p.nodes[3].pda];
    p.earn_income(INCOME);
    submit_rounds(&mut p, &gateway, &[d, d, d, a]);
    p.finalize_epoch(0);

    let state = p.env.epoch(0);
    let d_slot = usize::from(p.roster_slot_of(0, &d).unwrap());
    assert_eq!(state.weight(d_slot), 1);
    p.credit_all(0);
    assert!(claimable(&p, &d) > 0);
}

#[test]
fn credits_are_one_per_weighted_slot_and_only_after_finalization() {
    let (mut p, gateway) = setup(4);
    let [a, d] = [p.nodes[0].pda, p.nodes[3].pda];
    p.earn_income(INCOME);
    submit_rounds(&mut p, &gateway, &[d, d, d, d]);
    let a_slot = p.roster_slot_of(0, &a).unwrap();
    let d_slot = p.roster_slot_of(0, &d).unwrap();

    let failed = p
        .env
        .send_err(&[p.credit_epoch_rewards_ix(0, &[a_slot])], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotFinalized);

    p.finalize_epoch(0);
    // Zero weight.
    let failed = p
        .env
        .send_err(&[p.credit_epoch_rewards_ix(0, &[d_slot])], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToCredit);

    // Another node's account for the slot.
    let mut ix = p.credit_epoch_rewards_ix(0, &[a_slot]);
    ix.accounts.last_mut().unwrap().pubkey = p.nodes[1].pda;
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);

    // A read-only node account.
    let mut ix = p.credit_epoch_rewards_ix(0, &[a_slot]);
    ix.accounts.last_mut().unwrap().is_writable = false;
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);

    // The same slot twice in one batch, then again later.
    let failed = p
        .env
        .send_err(&[p.credit_epoch_rewards_ix(0, &[a_slot, a_slot])], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToCredit);
    p.env
        .send_ok(&[p.credit_epoch_rewards_ix(0, &[a_slot])], &[]);
    let failed = p
        .env
        .send_err(&[p.credit_epoch_rewards_ix(0, &[a_slot])], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToCredit);
    assert_eq!(p.env.epoch(0).credited_count, 1);
    p.assert_solvent();
}

/// A node tombstoned by slashing forfeits its uncredited share, which rolls into a later pool.
#[test]
fn a_tombstoned_node_forfeits_its_share_to_a_later_pool() {
    let (mut p, gateway) = setup(50);
    p.earn_income(INCOME);
    let missing = p.nodes[3].pda;
    submit_rounds(&mut p, &gateway, &[missing; 3]);
    p.finalize_epoch(0);
    let victim = p.nodes[0].pda;
    p.overwrite_node(&victim, |node| node.status = NodeStatus::Tombstoned);

    let slots = p.creditable_slots(0);
    let meta = p.env.send_ok(&[p.credit_epoch_rewards_ix(0, &slots)], &[]);
    let forfeited: Vec<_> = events::<EpochRewardCredited>(&meta)
        .into_iter()
        .filter(|event| event.forfeited)
        .collect();
    assert_eq!(forfeited.len(), 1);
    assert_eq!((forfeited[0].node, forfeited[0].amount), (victim, 0));
    assert_eq!(claimable(&p, &victim), 0);

    let state = p.env.epoch(0);
    let victim_share =
        state.payout(state.weight(usize::from(p.roster_slot_of(0, &victim).unwrap())));
    assert!(state.residual() >= victim_share);
    let liability = p.config().pool_liability;

    p.env.warp_to(Protocol::close_deadline(&state));
    let open = p.open_epoch();
    let open_pool = p.env.epoch(open).pool;
    let meta = p.env.send_ok(&[p.close_epoch_ix(0, open)], &[]);
    assert_eq!(p.env.epoch(open).pool, open_pool + state.residual());
    assert_eq!(
        p.config().pool_liability,
        liability,
        "rolling forward changes no liability"
    );
    let closed: EpochClosed = only_event(&meta);
    assert_eq!(
        (closed.epoch, closed.residual, closed.rolled_into),
        (0, state.residual(), open)
    );
    p.assert_solvent();
}

#[test]
fn close_waits_for_full_credit_and_the_evidence_window() {
    let (mut p, gateway) = setup(50);
    p.earn_income(INCOME);
    let missing = p.nodes[3].pda;
    submit_rounds(&mut p, &gateway, &[missing; 2]);
    p.finalize_epoch(0);
    let state = p.env.epoch(0);
    let deadline = Protocol::close_deadline(&state);

    p.env.warp_to(deadline);
    let open = p.open_epoch();
    let failed = p.env.send_err(&[p.close_epoch_ix(0, open)], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotClosable);

    p.credit_all(0);
    p.env.warp_to(deadline - 1);
    let open = p.open_epoch();
    let failed = p.env.send_err(&[p.close_epoch_ix(0, open)], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotClosable);

    p.env.warp_to(deadline);
    let open = p.open_epoch();
    // The residual must go to the epoch open now, not one already over.
    let failed = p.env.send_err(&[p.close_epoch_ix(0, open - 1)], &[]);
    assert_molpha_error(&failed, MolphaError::EpochNotOpen);
    // Rent returns to whoever created the epoch.
    let mut ix = p.close_epoch_ix(0, open);
    ix.accounts.last_mut().unwrap().pubkey = p.env.payer_pubkey();
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::InvalidRentReceiver);

    let rent_payer = Pubkey::new_from_array(state.rent_payer);
    assert_eq!(
        rent_payer,
        p.authority.pubkey(),
        "genesis epochs are paid by the authority"
    );
    let rent = p.env.lamports(&pda::epoch(0));
    let before = p.env.lamports(&rent_payer);
    p.env.send_ok(&[p.close_epoch_ix(0, open)], &[]);
    assert!(!p.env.account_exists(&pda::epoch(0)));
    assert_eq!(p.env.lamports(&rent_payer), before + rent);
    p.assert_solvent();
}

/// With no weighted node the whole pool rolls forward; nothing is lost.
#[test]
fn an_epoch_without_weight_rolls_its_whole_pool_forward() {
    let (mut p, _) = setup(50);
    p.earn_income(INCOME);
    let pool = p.env.epoch(0).pool;
    p.finalize_epoch(0);
    let state = p.env.epoch(0);
    assert_eq!((state.weight_total, state.weighted_count), (0, 0));

    p.env.warp_to(Protocol::close_deadline(&state));
    let open = p.open_epoch();
    p.env.send_ok(&[p.close_epoch_ix(0, open)], &[]);
    assert_eq!(p.env.epoch(open).pool, pool);
    assert_eq!(p.config().pool_liability, pool);
    p.assert_solvent();
}

/// Income, tickets and rewards across two epochs stay separate: each epoch pays its own pool.
#[test]
fn consecutive_epochs_pay_their_own_pools() {
    let (mut p, gateway) = setup(50);
    let first = p.run_reward_epoch(&gateway, INCOME, 3);
    let after_first: Vec<u64> = p.node_pdas().iter().map(|n| claimable(&p, n)).collect();
    let second = p.run_reward_epoch(&gateway, 2 * INCOME, 3);
    assert_eq!((first, second), (0, 1));

    let paid_second = p.env.epoch(1).paid;
    let total: u64 = p.node_pdas().iter().map(|n| claimable(&p, n)).sum();
    assert_eq!(total, after_first.iter().sum::<u64>() + paid_second);
    assert!(paid_second > p.env.epoch(0).paid);
    p.assert_solvent();
}
