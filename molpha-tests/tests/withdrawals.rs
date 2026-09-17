//! Node-deposit and gateway-bond withdrawal lifecycles.

use molpha::error::MolphaError;
use molpha::events::{GatewayBondWithdrawn, GatewayDeactivated, NodeDepositWithdrawn};
use molpha::instructions::RegisterGatewayArgs;
use molpha::state::{Gateway, GatewayStatus, Node, NodeStatus, PlanType};
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
    assert_molpha_error(&active, MolphaError::GatewayNotDeactivated);

    let stranger = p.env.keypair();
    let unauthorized = p.deactivate_gateway_ix(&stranger.pubkey(), &authority.pubkey());
    let failed = p.env.send_err(&[unauthorized], &[&stranger]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedGatewayAuthority);

    p.env.advance_seconds(1);
    let protocol_authority = p.authority.insecure_clone();
    let deactivate = p.deactivate_gateway_ix(&protocol_authority.pubkey(), &authority.pubkey());
    let meta = p.env.send_ok(&[deactivate], &[&protocol_authority]);
    let gateway_state: Gateway = p.env.get(&gateway);
    assert_eq!(gateway_state.status, GatewayStatus::Deactivated);
    assert_eq!(
        gateway_state.withdrawable_slot,
        p.env.slot() + p.args.dispute_window_slots + 1
    );
    assert_eq!(p.env.token_balance(&bond_ata), bond);

    let event: GatewayDeactivated = only_event(&meta);
    assert_eq!(event.gateway, gateway);
    assert_eq!(event.authority, authority.pubkey());
    assert_eq!(event.withdrawable_slot, gateway_state.withdrawable_slot);

    let mut unauthorized_withdraw = p.withdraw_gateway_bond_ix(&authority.pubkey());
    unauthorized_withdraw.accounts[0].pubkey = stranger.pubkey();
    let failed = p.env.send_err(&[unauthorized_withdraw], &[&stranger]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);

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

    p.fund_usdc(&p.authority.pubkey(), bond);
    let register = p.register_gateway_ix(
        &authority.pubkey(),
        &RegisterGatewayArgs {
            ip: [203, 0, 113, 9],
            port: 9_443,
            bond_amount: bond,
        },
    );
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[register], &[&protocol_authority]);
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

    let x402 = p.settle_x402_round_ix(
        &gateway.pubkey(),
        &signed.attestation(),
        p.x402_args(&signed, [0x71; 32]),
    );
    let failed = p.env.send_err(&[x402], &[&gateway_key]);
    assert_molpha_error(&failed, MolphaError::GatewayNotActive);

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
    assert_molpha_error(&failed, MolphaError::GatewayNotActive);
}

/// The point of the split vaults: a node's principal is not available to pay anyone's rewards.
#[test]
fn node_principal_is_never_available_to_pay_rewards() {
    let mut p = Protocol::builder().reward_per_signature(10_000_000).start();
    p.register_nodes("principal", 8);
    let gateway = p.register_gateway(500_000_000);

    p.add_plan(molpha::instructions::AddPlanArgs {
        plan_type: PlanType::Professional,
        subscription_price: 1,
        max_signers: 8,
        max_delegates: 1,
        max_rounds: 10,
        private_api_enabled: true,
        is_active: true,
    });
    let subscriber = p.subscribe(PlanType::Professional);

    // Eight deposits sit on chain, but none of them is treasury money.
    let deposit = p.args.minimum_node_deposit;
    assert_eq!(p.treasury_balance(), 1, "only the month's price is revenue");
    assert_eq!(p.node_deposit_balance(), 8 * deposit);
    assert_eq!(p.total_locked_deposits(), 8 * deposit);

    let signed = p.sign_duty(Duty::new(8, 4));
    let ixs = vec![
        p.consumer_authorization_ix(&subscriber, &gateway.pubkey(), &signed.duty),
        p.settle_subscription_round_ix(
            &gateway.pubkey(),
            &subscriber.pubkey(),
            &subscriber.pubkey(),
            None,
            &signed,
        ),
    ];
    let gateway_kp = gateway.insecure_clone();
    let subscriber_kp = subscriber.insecure_clone();
    p.env.send_ok(&ixs, &[&subscriber_kp, &gateway_kp]);

    let record = pda::subscription_round(
        &subscriber.pubkey(),
        &pda::gateway(&gateway.pubkey()),
        &signed.duty.source_id,
        signed.duty.signatures_required,
        signed.duty.canonical_timestamp,
    );
    let credited = p.node_pdas_for_bitmap(signed.duty.registry_version, &signed.signers_bitmap);
    p.env.advance_slots(p.args.dispute_window_slots + 1);

    // Crediting is refused outright rather than reaching for the deposits.
    let caller = p.env.keypair();
    let ix = p.finalize_round_ix(&caller.pubkey(), &record, &credited);
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::TreasuryInsolvent);

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
    assert_eq!(p.treasury_balance(), 1, "revenue was never touched");
    p.assert_solvent();
}

/// The vault is pinned by derivation from its own authority PDA, so no other token account can
/// stand in for it and drain deposits to an attacker-chosen destination.
#[test]
fn withdraw_rejects_a_deposit_vault_that_is_not_the_protocols() {
    let mut p = Protocol::start();
    p.register_nodes("foreign-vault", 2);
    let owner = p.nodes[0].owner.insecure_clone();
    p.env
        .send_ok(&[p.deactivate_node_ix(0, &owner.pubkey())], &[&owner]);
    p.env.advance_slots(p.args.withdrawal_cooldown_slots);

    // A token account of the same mint that the attacker controls.
    let impostor = p.env.keypair();
    let impostor_ata = p.fund_usdc(&impostor.pubkey(), p.args.minimum_node_deposit);

    let mut ix = p.withdraw_node_deposit_ix(&owner.pubkey());
    let slot = ix
        .accounts
        .iter()
        .position(|meta| meta.pubkey == p.node_deposit_vault())
        .expect("the vault is an account of this instruction");
    ix.accounts[slot].pubkey = impostor_ata;

    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintTokenOwner);
    assert_eq!(
        p.node_deposit_balance(),
        2 * p.args.minimum_node_deposit,
        "both deposits are untouched"
    );
}

/// The shared vault means the token program no longer bounds a payout to the payee's own
/// deposit, so the program does. This pins the bound at the two places that debit the vault.
#[test]
fn a_node_can_only_ever_draw_its_own_deposit_out_of_the_shared_vault() {
    let mut p = Protocol::start();
    p.register_nodes("shared-vault", 4);
    let deposit = p.args.minimum_node_deposit;
    assert_eq!(p.node_deposit_balance(), 4 * deposit);

    // Everyone deactivates and withdraws. The vault drains to exactly zero: no node can take more
    // than it put in, and the last one out is not left short.
    for index in 0..4 {
        let owner = p.nodes[index].owner.insecure_clone();
        let ix = p.deactivate_node_ix(index, &owner.pubkey());
        p.env.send_ok(&[ix], &[&owner]);
    }
    p.env.advance_slots(p.args.withdrawal_cooldown_slots);

    for index in 0..4 {
        let owner = p.nodes[index].owner.insecure_clone();
        let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
        let before = p.env.token_balance_or_zero(&destination);
        let ix = p.withdraw_node_deposit_ix(&owner.pubkey());
        p.env.send_ok(&[ix], &[&owner]);
        assert_eq!(p.env.token_balance(&destination), before + deposit);
        p.assert_solvent();
    }

    assert_eq!(p.node_deposit_balance(), 0);
    assert_eq!(p.total_locked_deposits(), 0);

    // A second attempt takes nothing, even though the vault is shared with nobody left in it.
    let owner = p.nodes[0].owner.insecure_clone();
    let failed = p
        .env
        .send_err(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToWithdraw);
}

/// A donation to the vault is not claimable by anyone: withdrawals pay `locked_amount`, not a
/// share of the balance.
#[test]
fn a_surplus_in_the_deposit_vault_is_not_withdrawable() {
    let mut p = Protocol::start();
    p.register_nodes("vault-surplus", 2);
    let deposit = p.args.minimum_node_deposit;
    p.fund_usdc(&pda::registry_state(), 500_000_000);
    assert_eq!(p.node_deposit_balance(), 2 * deposit + 500_000_000);
    assert_eq!(p.total_locked_deposits(), 2 * deposit);

    let owner = p.nodes[0].owner.insecure_clone();
    p.env
        .send_ok(&[p.deactivate_node_ix(0, &owner.pubkey())], &[&owner]);
    p.env.advance_slots(p.args.withdrawal_cooldown_slots);
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let before = p.env.token_balance_or_zero(&destination);
    p.env
        .send_ok(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);

    assert_eq!(
        p.env.token_balance(&destination),
        before + deposit,
        "the deposit, not a share of the vault"
    );
    p.assert_solvent();
}
