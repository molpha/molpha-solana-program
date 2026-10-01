//! Node-deposit withdrawal lifecycle and vault isolation.

use molpha::error::MolphaError;
use molpha::events::NodeDepositWithdrawn;
use molpha::state::{Node, NodeStatus, PlanConfig, PlanType};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

#[test]
fn a_deactivated_node_withdraws_its_deposit_at_the_exact_cooldown_boundary() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-node", 1);
    let owner = p.nodes[0].owner.insecure_clone();
    let node_pda = p.nodes[0].pda;
    let deposit = p.args.minimum_node_deposit;
    p.overwrite_node(&node_pda, |node| node.claimable_rewards = 77);

    let deactivate = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[deactivate], &[&owner]);

    let withdraw = p.withdraw_node_deposit_ix(&owner.pubkey());
    let early = p.env.send_err(std::slice::from_ref(&withdraw), &[&owner]);
    assert_molpha_error(&early, MolphaError::WithdrawalCooldownActive);

    p.env.advance_slots(p.args.withdrawal_cooldown_slots);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let balance_before = p.env.token_balance(&destination);
    let vault_before = p.node_deposit_balance();
    let treasury_before = p.treasury_balance();
    let meta = p.env.send_ok(&[withdraw], &[&owner]);

    // Principal comes out of the deposit vault; the treasury is not even an account here.
    assert_eq!(p.node_deposit_balance(), vault_before - deposit);
    assert_eq!(p.treasury_balance(), treasury_before);
    // No `assert_solvent` here: the fixture plants `claimable_rewards` directly, which by design
    // desynchronises the node from `reward_liability`.
    assert_eq!(p.total_locked_deposits(), p.node_deposit_balance());

    let node: Node = p.env.get(&node_pda);
    assert_eq!(node.status, NodeStatus::Deactivated);
    assert_eq!(node.locked_amount, 0);
    assert_eq!(
        node.claimable_rewards, 77,
        "deposit withdrawal is not a reward claim"
    );
    assert_eq!(p.env.token_balance(&destination), balance_before + deposit);

    let event: NodeDepositWithdrawn = only_event(&meta);
    assert_eq!(event.node, node_pda);
    assert_eq!(event.node_authority, owner.pubkey());
    assert_eq!(event.destination, destination);
    assert_eq!(event.amount, deposit);

    let repeated = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);
    assert_molpha_error(&repeated, MolphaError::NothingToWithdraw);
}

#[test]
fn active_and_frozen_nodes_cannot_withdraw_deposits() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-node-state", 2);

    let active = p.nodes[0].owner.insecure_clone();
    let failed = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&active.pubkey())], &[&active]);
    assert_molpha_error(&failed, MolphaError::InvalidWithdrawalState);

    let frozen = p.nodes[1].owner.insecure_clone();
    let node_pda = p.nodes[1].pda;
    p.overwrite_node(&node_pda, |node| {
        node.status = NodeStatus::Frozen;
        node.frozen_until = i64::MAX;
    });
    let failed = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&frozen.pubkey())], &[&frozen]);
    assert_molpha_error(&failed, MolphaError::InvalidWithdrawalState);
}

#[test]
fn node_withdrawal_requires_the_owner_and_its_canonical_usdc_account() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-node-authority", 1);
    let owner = p.nodes[0].owner.insecure_clone();
    let stranger = p.env.keypair();
    p.fund_usdc(&stranger.pubkey(), 1);

    let deactivate = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[deactivate], &[&owner]);
    p.env.advance_slots(p.args.withdrawal_cooldown_slots);

    let mut wrong_authority = p.withdraw_node_deposit_ix(&owner.pubkey());
    wrong_authority.accounts[0].pubkey = stranger.pubkey();
    let failed = p.env.send_err(&[wrong_authority], &[&stranger]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);

    let mut wrong_destination = p.withdraw_node_deposit_ix(&owner.pubkey());
    let canonical = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let stranger_ata = pda::ata(&stranger.pubkey(), &p.usdc_mint);
    let destination = wrong_destination
        .accounts
        .iter_mut()
        .find(|meta| meta.pubkey == canonical)
        .expect("withdraw instruction has destination ATA");
    destination.pubkey = stranger_ata;
    let failed = p.env.send_err(&[wrong_destination], &[&owner]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);
}

#[test]
fn a_tombstoned_node_withdraws_after_its_cooldown_slot() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-legacy-tombstone", 1);
    let owner = p.nodes[0].owner.insecure_clone();
    let node_pda = p.nodes[0].pda;
    let survivor = p.args.minimum_node_deposit / 2;
    let withdrawable_slot = p
        .env
        .slot()
        .saturating_add(p.args.withdrawal_cooldown_slots);
    p.overwrite_node(&node_pda, |node| {
        node.status = NodeStatus::Tombstoned;
        node.locked_amount = survivor;
        node.withdrawable_slot = withdrawable_slot;
    });

    let early = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);
    assert_molpha_error(&early, MolphaError::WithdrawalCooldownActive);

    p.env.advance_slots(p.args.withdrawal_cooldown_slots);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance(&destination);
    let withdraw = p.withdraw_node_deposit_ix(&owner.pubkey());
    p.env.send_ok(&[withdraw], &[&owner]);
    assert_eq!(p.env.token_balance(&destination), before + survivor);
}

#[test]
fn a_deactivated_gateway_can_no_longer_admit_tickets() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-gateway-tickets", 8);
    let gateway = p.register_gateway();
    let signed = p.sign_canonical(p.live_duty(4));

    let protocol_authority = p.authority.insecure_clone();
    let deactivate = p.deactivate_gateway_ix(&protocol_authority.pubkey(), &gateway.pubkey());
    p.env.send_ok(&[deactivate], &[&protocol_authority]);

    let submitter = p.env.keypair();
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::GatewayNotActive);
    assert_eq!(p.env.epoch(0).tickets, 0);
}

/// The point of the split vaults: a node's principal is never income, never a pool and never
/// pays anyone's rewards. Pools pay out only income that arrived.
#[test]
fn node_principal_is_never_available_to_pay_rewards() {
    let mut p = Protocol::start();
    p.register_nodes("principal", 8);
    let gateway = p.register_gateway();

    p.add_plan(PlanConfig {
        plan_type: PlanType::Professional,
        subscription_price: 1_000,
        max_signers: 8,
        max_delegates: 1,
        max_rounds: 10,
        private_api_enabled: true,
        is_active: true,
    });
    p.subscribe(PlanType::Professional);

    // Eight deposits sit on chain, but none of them is treasury money.
    let deposit = p.args.minimum_node_deposit;
    assert_eq!(
        p.treasury_balance(),
        1_000,
        "only the month's price is revenue"
    );
    assert_eq!(p.node_deposit_balance(), 8 * deposit);

    // Recognize the subscription as income, then pay it out through a full epoch.
    let epoch = p.open_epoch();
    p.env.send_ok(&[p.sweep_treasury_revenue_ix(epoch)], &[]);
    let pool = p.env.epoch(epoch).pool;
    assert_eq!(
        pool,
        1_000 - 1_000 * u64::from(p.args.protocol_fee_bps) / 10_000
    );
    p.run_reward_epoch(&gateway, 0, 4);

    let credited: u64 = p
        .node_pdas()
        .iter()
        .map(|node| p.env.get::<Node>(node).claimable_rewards)
        .sum();
    assert!(
        credited <= pool,
        "payouts never exceed the income that arrived"
    );
    assert_eq!(
        p.node_deposit_balance(),
        8 * deposit,
        "deposits were never touched"
    );

    // Every deposit is still whole and still withdrawable in full.
    let owner = p.nodes[0].owner.insecure_clone();
    p.env
        .send_ok(&[p.deactivate_node_ix(0, &owner.pubkey())], &[&owner]);
    p.env.advance_slots(p.args.withdrawal_cooldown_slots);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance_or_zero(&destination);
    p.env
        .send_ok(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);

    assert_eq!(p.env.token_balance(&destination), before + deposit);
    assert_eq!(p.node_deposit_balance(), 7 * deposit);
    assert_eq!(
        p.treasury_balance(),
        1_000,
        "revenue never left the treasury"
    );
    p.assert_solvent();
}
