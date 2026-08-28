//! Node-deposit and gateway-bond withdrawal lifecycles.

use molpha::error::MolphaError;
use molpha::events::{GatewayBondWithdrawn, GatewayDeactivated, NodeDepositWithdrawn};
use molpha::instructions::RegisterGatewayArgs;
use molpha::state::{Gateway, Node, NodeStatus, PlanType};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
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

    p.env.advance_seconds(p.args.withdrawal_cooldown_secs);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let balance_before = p.env.token_balance(&destination);
    let meta = p.env.send_ok(&[withdraw], &[&owner]);

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
    assert_eq!(event.owner, owner.pubkey());
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
    p.env.advance_seconds(p.args.withdrawal_cooldown_secs);

    let mut wrong_authority = p.withdraw_node_deposit_ix(&owner.pubkey());
    wrong_authority.accounts[0].pubkey = stranger.pubkey();
    let failed = p.env.send_err(&[wrong_authority], &[&stranger]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);

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
fn a_legacy_tombstone_without_a_snapshot_uses_punished_at_plus_current_cooldown() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-legacy-tombstone", 1);
    let owner = p.nodes[0].owner.insecure_clone();
    let node_pda = p.nodes[0].pda;
    let survivor = p.args.minimum_node_deposit / 2;
    let timestamp = p
        .env
        .unix_timestamp()
        .checked_add(p.args.withdrawal_cooldown_secs)
        .unwrap();
    p.overwrite_node(&node_pda, |node| {
        node.status = NodeStatus::Tombstoned;
        node.locked_amount = survivor;
        node.withdrawable_at = timestamp;
    });

    let early = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);
    assert_molpha_error(&early, MolphaError::WithdrawalCooldownActive);

    p.env.advance_seconds(p.args.withdrawal_cooldown_secs);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance(&destination);
    let withdraw = p.withdraw_node_deposit_ix(&owner.pubkey());
    p.env.send_ok(&[withdraw], &[&owner]);
    assert_eq!(p.env.token_balance(&destination), before + survivor);
}

#[test]
fn a_gateway_exit_waits_for_the_dispute_window_and_keeps_its_identity_account() {
    let mut p = Protocol::start();
    let gateway_authority = p.register_gateway(0);
    let authority = gateway_authority.insecure_clone();
    let gateway = pda::gateway(&authority.pubkey());
    let bond_ata = pda::ata(&gateway, &p.usdc_mint);
    let bond = p.args.gateway_bond_min;

    let active = p.env.send_err(
        &[p.withdraw_gateway_bond_ix(&authority.pubkey())],
        &[&authority],
    );
    assert_molpha_error(&active, MolphaError::GatewayActive);

    let stranger = p.env.keypair();
    let unauthorized = p.deactivate_gateway_ix(&stranger.pubkey(), &authority.pubkey());
    let failed = p.env.send_err(&[unauthorized], &[&stranger]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);

    p.env.advance_seconds(1);
    let protocol_authority = p.authority.insecure_clone();
    let deactivate = p.deactivate_gateway_ix(&protocol_authority.pubkey(), &authority.pubkey());
    let meta = p.env.send_ok(&[deactivate], &[&protocol_authority]);
    let gateway_state: Gateway = p.env.get(&gateway);
    assert_eq!(gateway_state.deactivated_at, p.env.unix_timestamp());
    assert_eq!(
        gateway_state.withdrawable_slot,
        p.env.slot() + p.args.dispute_window_slots + 1
    );
    assert_eq!(p.env.token_balance(&bond_ata), bond);

    let event: GatewayDeactivated = only_event(&meta);
    assert_eq!(event.gateway, gateway);
    assert_eq!(event.authority, authority.pubkey());
    assert_eq!(event.deactivated_at, gateway_state.deactivated_at);
    assert_eq!(event.withdrawable_slot, gateway_state.withdrawable_slot);

    let mut unauthorized_withdraw = p.withdraw_gateway_bond_ix(&authority.pubkey());
    unauthorized_withdraw.accounts[0].pubkey = stranger.pubkey();
    let failed = p.env.send_err(&[unauthorized_withdraw], &[&stranger]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);

    let slot_blocked = p.env.send_err(
        &[p.withdraw_gateway_bond_ix(&authority.pubkey())],
        &[&authority],
    );
    assert_molpha_error(&slot_blocked, MolphaError::GatewayDisputeWindowActive);

    let remaining_slots = gateway_state.withdrawable_slot - p.env.slot();
    p.env.advance_slots(remaining_slots);
    let destination = pda::ata(&authority.pubkey(), &p.usdc_mint);
    let balance_before = p.env.token_balance(&destination);
    let lamports_before = p.env.lamports(&authority.pubkey());
    let meta = p.env.send_ok(
        &[p.withdraw_gateway_bond_ix(&authority.pubkey())],
        &[&authority],
    );

    assert_eq!(p.env.token_balance(&destination), balance_before + bond);
    assert!(p.env.account_exists(&gateway));
    assert!(!p.env.account_exists(&bond_ata));
    assert!(p.env.lamports(&authority.pubkey()) > lamports_before);

    let withdrawn: GatewayBondWithdrawn = only_event(&meta);
    assert_eq!(withdrawn.gateway, gateway);
    assert_eq!(withdrawn.amount, bond);

    let register = p.register_gateway_ix(
        &authority.pubkey(),
        &RegisterGatewayArgs {
            ip: [203, 0, 113, 9],
            port: 9_443,
            bond_amount: bond,
        },
    );
    let failed = p.env.send_err(&[register], &[&authority]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|line| line.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn deactivating_a_gateway_disables_both_settlement_paths() {
    let mut p = Protocol::start();
    p.register_nodes("withdraw-gateway-settlement", 8);
    let gateway = p.register_gateway(500_000_000);
    let gateway_key = gateway.insecure_clone();
    let duty = Duty::new(p.current_version(), 4);
    let signed = p.sign_duty(duty);

    p.env.advance_seconds(1);
    let protocol_authority = p.authority.insecure_clone();
    let deactivate = p.deactivate_gateway_ix(&protocol_authority.pubkey(), &gateway.pubkey());
    p.env.send_ok(&[deactivate], &[&protocol_authority]);

    let x402 = p.settle_x402_round_ix(&gateway.pubkey(), p.x402_args(&signed, [0x71; 32]));
    let failed = p.env.send_err(&[x402], &[&gateway_key]);
    assert_molpha_error(&failed, MolphaError::GatewayDeactivated);

    p.add_default_plan(PlanType::Professional);
    let subscriber = p.subscribe(PlanType::Professional);
    let subscriber_key = subscriber.insecure_clone();
    let authorize = p.consumer_authorization_ix(&subscriber, &gateway.pubkey(), &duty);
    let settle = p.settle_subscription_round_ix(
        &gateway.pubkey(),
        &subscriber.pubkey(),
        &subscriber.pubkey(),
        None,
        &signed,
    );
    let failed = p
        .env
        .send_err(&[authorize, settle], &[&subscriber_key, &gateway_key]);
    assert_molpha_error(&failed, MolphaError::GatewayDeactivated);
}
