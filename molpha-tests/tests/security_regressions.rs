//! Transaction-level attack cases. Every rejection pins an error and all named accounts.

use anchor_lang::error::ErrorCode;
use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountSerialize;
use molpha::error::MolphaError;
use molpha::events::{EpochRewardCredited, OffenceProven, TicketRosterOverflow};
use molpha::instructions::{AddDelegateArgs, MemberEndpoint};
use molpha::state::{
    ticket_id, EquivocationEvidence, Evidence, Feed, Node, PlanType, Subscription,
};
use molpha_tests::crypto;
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, events, only_event, TestEnv};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use molpha_tests::security::{account_meta_mut, AccountSnapshot};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

fn setup() -> (Protocol, Keypair, Keypair) {
    let mut p = Protocol::builder()
        .with_env(TestEnv::from_seed([71; 32]))
        .start();
    p.register_nodes("security-regressions", 4);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    (p, gateway, submitter)
}

fn rewards() -> (Protocol, Keypair) {
    let (mut p, gateway, _) = setup();
    p.run_reward_epoch(&gateway, 10_000_003, 2);
    (p, gateway)
}

fn equivocation(p: &Protocol, source: u8) -> EquivocationEvidence {
    let duty = p.live_duty(3).source_id([source; 32]);
    let a = p.sign_canonical(duty.value([1; 32]));
    let b = p.sign_canonical(duty.value([2; 32]));
    EquivocationEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    }
}

/// Rewrite classic SPL-token state to model external custody failures. These fixtures are
/// defense-in-depth: no Molpha instruction is able to forge a token account balance or state.
fn set_token_amount(p: &mut Protocol, account: &Pubkey, amount: u64) {
    let mut stored = p
        .env
        .svm
        .get_account(account)
        .expect("token account exists");
    stored.data[64..72].copy_from_slice(&amount.to_le_bytes());
    p.env.svm.set_account(*account, stored).unwrap();
}

fn freeze_token_account(p: &mut Protocol, account: &Pubkey) {
    let mut stored = p
        .env
        .svm
        .get_account(account)
        .expect("token account exists");
    stored.data[108] = 2; // spl_token::state::AccountState::Frozen
    p.env.svm.set_account(*account, stored).unwrap();
}

#[test]
fn registration_failure_after_deposit_transfer_restores_every_created_account_and_token() {
    let mut p = Protocol::start();
    let owner = p.env.keypair();
    p.fund_usdc(&owner.pubkey(), p.args.minimum_node_deposit);
    let key = crypto::node_keys("late-registration-rollback", 1).remove(0);
    let args = p.register_node_args(&owner.pubkey(), &key, [7; 32], 8_080);
    let authority = p.authority.insecure_clone();

    // Defense-in-depth corrupted state: validation prevents this delay from being stored. It
    // makes get_grace_active_until fail only after the deposit CPI and account initialization.
    p.overwrite_protocol_config(|config| config.max_settlement_delay_seconds = u64::MAX);
    let ix = p.register_node_ix(&owner.pubkey(), &args, &authority.pubkey());
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&authority, &owner]);
    assert_molpha_error(&failed, MolphaError::InvalidMaxReceiptDelay);
    before.assert_unchanged(&p.env);
}

#[test]
fn node_registration_requires_both_deposit_accounts_without_leaving_rent_accounts() {
    let mut p = Protocol::start();
    let owner = p.env.keypair();
    p.fund_usdc(&owner.pubkey(), p.args.minimum_node_deposit);
    let key = crypto::node_keys("missing-deposit-accounts", 1).remove(0);
    let args = p.register_node_args(&owner.pubkey(), &key, [8; 32], 8_081);
    let authority = p.authority.insecure_clone();
    let mut ix = p.register_node_ix(&owner.pubkey(), &args, &authority.pubkey());
    account_meta_mut(&mut ix, &pda::ata(&owner.pubkey(), &p.usdc_mint)).pubkey = molpha::ID;
    account_meta_mut(&mut ix, &p.node_deposit_vault()).pubkey = molpha::ID;
    let before = AccountSnapshot::capture(&p.env, &[ix.clone()], &[]);
    let failed = p.env.send_err(&[ix], &[&authority, &owner]);
    assert_molpha_error(&failed, MolphaError::NodeDepositAccountsMissing);
    before.assert_unchanged(&p.env);
}

#[test]
fn a_late_invalid_credit_rolls_back_the_entire_valid_prefix() {
    let (mut p, _) = rewards();
    let slots = p.creditable_slots(0);
    // run_reward_epoch credits everything; use a fresh epoch with uncredited weights.
    assert!(slots.is_empty());
    let gateway = p.register_gateway();
    p.earn_income(10_000_001);
    let actor = p.env.keypair();
    let signed = p.sign_canonical(p.live_duty(3).source_id([55; 32]));
    p.submit_ticket(&actor, &gateway, &signed);
    p.finalize_epoch(1);
    let slots = p.creditable_slots(1);
    assert!(slots.len() >= 2);
    let mut ix = p.credit_epoch_rewards_ix(1, &slots[..2]);
    ix.accounts.last_mut().unwrap().pubkey = p.nodes[3].pda;
    let snapshot = AccountSnapshot::capture(&p.env, &[ix.clone()], &p.node_pdas());
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);
    snapshot.assert_unchanged(&p.env);
    assert_eq!(p.env.epoch(1).credited_count, 0);
    p.env
        .send_ok(&[p.credit_epoch_rewards_ix(1, &slots[..2])], &[]);
    p.assert_solvent();
}

#[test]
fn duplicate_credit_in_one_batch_restores_rewards_and_the_credit_bitmap() {
    let (mut p, gateway, submitter) = setup();
    p.earn_income(101);
    let signed = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &signed);
    p.finalize_epoch(0);
    let slot = p.creditable_slots(0)[0];
    let ix = p.credit_epoch_rewards_ix(0, &[slot, slot]);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToCredit);
    before.assert_unchanged(&p.env);
    p.assert_solvent();
}

#[test]
fn empty_and_mismatched_credit_batches_do_not_mutate_the_epoch() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &signed);
    p.finalize_epoch(0);
    let slot = p.creditable_slots(0)[0];
    let empty = p.credit_epoch_rewards_ix(0, &[]);
    let mut short = p.credit_epoch_rewards_ix(0, &[slot]);
    short.accounts.pop();
    for ix in [empty, short] {
        let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
        let failed = p.env.send_err(&[ix], &[]);
        assert_molpha_error(&failed, MolphaError::InvalidRosterSlot);
        before.assert_unchanged(&p.env);
    }
}

fn uncredited_epoch() -> (Protocol, Vec<u16>) {
    let (mut p, gateway, submitter) = setup();
    p.earn_income(10_000_003);
    let signed = p.sign_canonical(p.live_duty(3).source_id([54; 32]));
    p.submit_ticket(&submitter, &gateway, &signed);
    p.finalize_epoch(0);
    let slots = p.creditable_slots(0);
    assert!(slots.len() >= 2);
    (p, slots)
}

#[test]
fn reward_credit_overflow_or_liability_underflow_restores_the_valid_batch_prefix() {
    let (mut overflow, slots) = uncredited_epoch();
    let second = Pubkey::new_from_array(overflow.env.epoch(0).roster[usize::from(slots[1])]);
    // Defense-in-depth corrupted accounting: normal crediting cannot create this node balance.
    overflow.overwrite_node(&second, |node| node.claimable_rewards = u64::MAX);
    let ix = overflow.credit_epoch_rewards_ix(0, &slots[..2]);
    let before = AccountSnapshot::capture(
        &overflow.env,
        std::slice::from_ref(&ix),
        &overflow.node_pdas(),
    );
    let failed = overflow.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::RewardOverflow);
    before.assert_unchanged(&overflow.env);

    let (mut underflow, slots) = uncredited_epoch();
    let state = underflow.env.epoch(0);
    let first_share = state.payout(state.weight(usize::from(slots[0])));
    assert!(first_share > 0);
    // Defense-in-depth corrupted liability: the first credit consumes it and the second detects
    // the underflow after the first node and epoch bitmap were already written.
    underflow.overwrite_protocol_config(|config| config.pool_liability = first_share);
    let ix = underflow.credit_epoch_rewards_ix(0, &slots[..2]);
    let before = AccountSnapshot::capture(
        &underflow.env,
        std::slice::from_ref(&ix),
        &underflow.node_pdas(),
    );
    let failed = underflow.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::RewardLiabilityUnderflow);
    before.assert_unchanged(&underflow.env);
}

#[test]
fn an_empty_or_repeated_claim_preserves_all_custody_accounts() {
    let (mut p, _) = rewards();
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.claim_node_rewards_ix(&owner.pubkey());
    p.env.send_ok(std::slice::from_ref(&ix), &[&owner]);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NothingToClaim);
    before.assert_unchanged(&p.env);
    p.assert_solvent();
}

#[test]
fn a_frozen_reward_destination_rolls_back_the_liability_and_credit() {
    let (mut p, _) = rewards();
    let owner = p.nodes[0].owner.insecure_clone();
    let destination = pda::ata(&owner.pubkey(), &p.usdc_mint);
    // External SPL account state: models a mint authority freezing the receiving account.
    freeze_token_account(&mut p, &destination);
    let ix = p.claim_node_rewards_ix(&owner.pubkey());
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(1, InstructionError::Custom(17))
    );
    before.assert_unchanged(&p.env);
    p.assert_solvent();
}

#[test]
fn frozen_reserve_and_deposit_destinations_roll_back_accounting_and_balances() {
    let (mut reserve_protocol, _) = rewards();
    let authority = reserve_protocol.authority.insecure_clone();
    let reserve_destination = reserve_protocol.fund_usdc(&authority.pubkey(), 0);
    freeze_token_account(&mut reserve_protocol, &reserve_destination);
    let amount = reserve_protocol.config().protocol_reserved;
    assert!(amount > 0);
    let ix = reserve_protocol.withdraw_protocol_reserve_ix(
        &authority.pubkey(),
        &reserve_destination,
        amount,
    );
    let before = AccountSnapshot::capture(&reserve_protocol.env, std::slice::from_ref(&ix), &[]);
    let failed = reserve_protocol.env.send_err(&[ix], &[&authority]);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(1, InstructionError::Custom(17))
    );
    before.assert_unchanged(&reserve_protocol.env);

    let mut deposit_protocol = Protocol::start();
    deposit_protocol.register_nodes("frozen-deposit-destination", 1);
    let owner = deposit_protocol.nodes[0].owner.insecure_clone();
    deposit_protocol.env.send_ok(
        &[deposit_protocol.deactivate_node_ix(0, &owner.pubkey())],
        &[&owner],
    );
    let node: Node = deposit_protocol.env.get(&deposit_protocol.nodes[0].pda);
    deposit_protocol
        .env
        .advance_slots(node.withdrawable_slot - deposit_protocol.env.slot());
    let deposit_destination = pda::ata(&owner.pubkey(), &deposit_protocol.usdc_mint);
    freeze_token_account(&mut deposit_protocol, &deposit_destination);
    let ix = deposit_protocol.withdraw_node_deposit_ix(&owner.pubkey());
    let before = AccountSnapshot::capture(&deposit_protocol.env, std::slice::from_ref(&ix), &[]);
    let failed = deposit_protocol.env.send_err(&[ix], &[&owner]);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(1, InstructionError::Custom(17))
    );
    before.assert_unchanged(&deposit_protocol.env);
}

#[test]
fn substituted_treasury_and_deposit_vaults_are_rejected_before_custody_changes() {
    let (mut reserve_protocol, _) = rewards();
    let authority = reserve_protocol.authority.insecure_clone();
    let destination = reserve_protocol.fund_usdc(&authority.pubkey(), 0);
    let stranger = reserve_protocol.env.keypair();
    let substitute = reserve_protocol.fund_usdc(&stranger.pubkey(), 0);
    let mut ix = reserve_protocol.withdraw_protocol_reserve_ix(
        &authority.pubkey(),
        &destination,
        reserve_protocol.config().protocol_reserved,
    );
    account_meta_mut(&mut ix, &reserve_protocol.treasury()).pubkey = substitute;
    let before = AccountSnapshot::capture(&reserve_protocol.env, &[ix.clone()], &[]);
    let failed = reserve_protocol.env.send_err(&[ix], &[&authority]);
    assert_anchor_error(&failed, ErrorCode::ConstraintTokenOwner);
    before.assert_unchanged(&reserve_protocol.env);

    let mut deposit_protocol = Protocol::start();
    deposit_protocol.register_nodes("substituted-deposit-vault", 1);
    let owner = deposit_protocol.nodes[0].owner.insecure_clone();
    deposit_protocol.env.send_ok(
        &[deposit_protocol.deactivate_node_ix(0, &owner.pubkey())],
        &[&owner],
    );
    let node: Node = deposit_protocol.env.get(&deposit_protocol.nodes[0].pda);
    deposit_protocol
        .env
        .advance_slots(node.withdrawable_slot - deposit_protocol.env.slot());
    let stranger = deposit_protocol.env.keypair();
    let substitute = deposit_protocol.fund_usdc(&stranger.pubkey(), 0);
    let mut ix = deposit_protocol.withdraw_node_deposit_ix(&owner.pubkey());
    account_meta_mut(&mut ix, &deposit_protocol.node_deposit_vault()).pubkey = substitute;
    let before = AccountSnapshot::capture(&deposit_protocol.env, &[ix.clone()], &[]);
    let failed = deposit_protocol.env.send_err(&[ix], &[&owner]);
    assert_anchor_error(&failed, ErrorCode::ConstraintTokenOwner);
    before.assert_unchanged(&deposit_protocol.env);
}

#[test]
fn claims_reject_substituted_destinations_and_token_programs() {
    let (mut p, _) = rewards();
    let owner = p.nodes[0].owner.insecure_clone();
    let stranger = p.env.keypair();
    let other = p.fund_usdc(&stranger.pubkey(), 0);
    let mut destination = p.claim_node_rewards_ix(&owner.pubkey());
    account_meta_mut(&mut destination, &pda::ata(&owner.pubkey(), &p.usdc_mint)).pubkey = other;
    let before = AccountSnapshot::capture(&p.env, &[destination.clone()], &[]);
    let failed = p.env.send_err(&[destination], &[&owner]);
    assert_anchor_error(&failed, ErrorCode::ConstraintTokenOwner);
    before.assert_unchanged(&p.env);
    let mut program = p.claim_node_rewards_ix(&owner.pubkey());
    account_meta_mut(&mut program, &pda::TOKEN_PROGRAM_ID).pubkey = anchor_lang::system_program::ID;
    let before = AccountSnapshot::capture(&p.env, &[program.clone()], &[]);
    let failed = p.env.send_err(&[program], &[&owner]);
    assert_anchor_error(&failed, ErrorCode::InvalidProgramId);
    before.assert_unchanged(&p.env);
}

#[test]
fn claim_requires_a_runtime_signature_and_an_anchor_signer() {
    let (mut p, _) = rewards();
    let owner = p.nodes[0].owner.insecure_clone();
    let mut ix = p.claim_node_rewards_ix(&owner.pubkey());
    let before = AccountSnapshot::capture(&p.env, &[ix.clone()], &[]);
    let failed = p.env.send_partially_signed(&[ix.clone()], &[]).unwrap_err();
    assert_eq!(failed.err, TransactionError::SignatureFailure);
    before.assert_unchanged(&p.env);
    account_meta_mut(&mut ix, &owner.pubkey()).is_signer = false;
    let failed = p.env.send_err(&[ix], &[]);
    assert_anchor_error(&failed, ErrorCode::AccountNotSigner);
    before.assert_unchanged(&p.env);
}

#[test]
fn underfunded_treasury_and_deposit_vault_guards_preserve_recorded_liabilities() {
    let (mut reward_protocol, _) = rewards();
    let owner = reward_protocol.nodes[0].owner.insecure_clone();
    let rewards: Node = reward_protocol.env.get(&reward_protocol.nodes[0].pda);
    assert!(rewards.claimable_rewards > 0);
    let treasury = reward_protocol.treasury();
    set_token_amount(
        &mut reward_protocol,
        &treasury,
        rewards.claimable_rewards - 1,
    );
    let ix = reward_protocol.claim_node_rewards_ix(&owner.pubkey());
    let before = AccountSnapshot::capture(&reward_protocol.env, std::slice::from_ref(&ix), &[]);
    let failed = reward_protocol.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::TreasuryInsolvent);
    before.assert_unchanged(&reward_protocol.env);

    let mut deposit_protocol = Protocol::start();
    deposit_protocol.register_nodes("underfunded-deposit-vault", 1);
    let owner = deposit_protocol.nodes[0].owner.insecure_clone();
    deposit_protocol.env.send_ok(
        &[deposit_protocol.deactivate_node_ix(0, &owner.pubkey())],
        &[&owner],
    );
    let node: Node = deposit_protocol.env.get(&deposit_protocol.nodes[0].pda);
    deposit_protocol
        .env
        .advance_slots(node.withdrawable_slot - deposit_protocol.env.slot());
    let vault = deposit_protocol.node_deposit_vault();
    // Defense-in-depth external custody corruption: the program only ever transfers the exact
    // recorded principal into or out of this canonical vault.
    set_token_amount(&mut deposit_protocol, &vault, node.locked_amount - 1);
    let ix = deposit_protocol.withdraw_node_deposit_ix(&owner.pubkey());
    let before = AccountSnapshot::capture(&deposit_protocol.env, std::slice::from_ref(&ix), &[]);
    let failed = deposit_protocol.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::DepositVaultUnderfunded);
    before.assert_unchanged(&deposit_protocol.env);
}

#[test]
fn reserve_withdrawal_zero_and_one_over_do_not_touch_other_liabilities() {
    let (mut p, _) = rewards();
    let authority = p.authority.insecure_clone();
    let destination = p.fund_usdc(&authority.pubkey(), 0);
    let reserve = p.config().protocol_reserved;
    for amount in [0, reserve + 1] {
        let ix = p.withdraw_protocol_reserve_ix(&authority.pubkey(), &destination, amount);
        let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &p.node_pdas());
        let failed = p.env.send_err(&[ix], &[&authority]);
        assert_molpha_error(&failed, MolphaError::InsufficientProtocolReserve);
        before.assert_unchanged(&p.env);
    }
    p.assert_solvent();
}

#[test]
fn a_hundred_percent_fee_leaves_no_pool_and_is_not_swept_twice() {
    let mut p = Protocol::builder().protocol_fee_bps(10_000).start();
    p.earn_income(1);
    assert_eq!(p.env.epoch(0).pool, 0);
    assert_eq!(p.config().protocol_reserved, 1);
    let ix = p.sweep_treasury_revenue_ix(0);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToSweep);
    before.assert_unchanged(&p.env);
    p.assert_solvent();
}

#[test]
fn a_second_instruction_failure_reverts_an_earlier_successful_sweep() {
    let mut p = Protocol::start();
    p.fund_treasury(101);
    let ixs = [
        p.sweep_treasury_revenue_ix(0),
        p.sweep_treasury_revenue_ix(0),
    ];
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[]);
    assert_molpha_error(&failed, MolphaError::NothingToSweep);
    before.assert_unchanged(&p.env);
    p.env.send_ok(&[ixs[0].clone()], &[]);
    p.assert_solvent();
}

#[test]
fn credited_rewards_are_never_recognized_as_new_income() {
    let (mut p, _) = rewards();
    assert!(p.reward_liability() > 0);
    let ix = p.sweep_treasury_revenue_ix(p.open_epoch());
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::NothingToSweep);
    before.assert_unchanged(&p.env);
    p.assert_solvent();
}

#[test]
fn slashing_after_credit_releases_only_the_guilty_rewards() {
    let (mut p, _) = rewards();
    let reporter = p.reporter();
    let evidence = equivocation(&p, 77);
    let bits = crypto::bitmap_bits(&evidence.attestation_a.signature.signers_bitmap);
    let guilty = p.node_pdas_for_bitmap(
        p.current_version(),
        &evidence.attestation_a.signature.signers_bitmap,
    );
    let rewards: u64 = guilty
        .iter()
        .map(|key| p.env.get::<Node>(key).claimable_rewards)
        .sum();
    assert!(rewards > 0);
    let liability = p.reward_liability();
    let reserve = p.config().protocol_reserved;
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let result: OffenceProven = only_event(&meta);
    assert_eq!(
        result.slashed_total,
        p.args.minimum_node_deposit * bits.len() as u64 + rewards
    );
    assert_eq!(p.reward_liability(), liability - rewards);
    assert_eq!(
        p.config().protocol_reserved,
        reserve + result.slashed_total - result.bounty_paid
    );
    p.assert_solvent();
}

#[test]
fn slashing_before_credit_forfeits_weight_without_moving_the_pool() {
    let (mut p, gateway, submitter) = setup();
    let evidence = equivocation(&p, 78);
    p.earn_income(1_001);
    let signed = p.sign_canonical(p.live_duty(3).source_id([79; 32]));
    p.submit_ticket(&submitter, &gateway, &signed);
    p.finalize_epoch(0);
    let pool_liability = p.config().pool_liability;
    let reporter = p.reporter();
    p.env.send_ok(
        &[p.slash_equivocation_ix(&reporter.pubkey(), evidence)],
        &[&reporter],
    );
    let slots = p.creditable_slots(0);
    let meta = p.env.send_ok(&[p.credit_epoch_rewards_ix(0, &slots)], &[]);
    assert!(events::<EpochRewardCredited>(&meta)
        .iter()
        .all(|event| event.forfeited && event.amount == 0));
    assert_eq!(p.config().pool_liability, pool_liability);
    p.assert_solvent();
}

#[test]
fn a_wrong_late_guilty_account_rolls_back_prior_punishments_and_rent() {
    let (mut p, _, _) = setup();
    let reporter = p.reporter();
    let evidence = equivocation(&p, 80);
    let case = pda::punishment_case(&evidence.hash());
    let mut ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence.clone());
    ix.accounts.last_mut().unwrap().pubkey = p.nodes[3].pda;
    let before = AccountSnapshot::capture(&p.env, &[ix.clone()], &p.node_pdas());
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);
    before.assert_unchanged(&p.env);
    assert!(!p.env.account_exists(&case));
    p.env.send_ok(
        &[p.slash_equivocation_ix(&reporter.pubkey(), evidence)],
        &[&reporter],
    );
    p.assert_solvent();
}

#[test]
fn reordered_and_duplicate_guilty_accounts_never_punish_an_innocent_node() {
    let (mut p, _, _) = setup();
    let reporter = p.reporter();
    let evidence = equivocation(&p, 81);
    let good = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let n = good.accounts.len();
    for duplicate in [false, true] {
        let mut ix = good.clone();
        if duplicate {
            ix.accounts[n - 1] = ix.accounts[n - 2].clone();
        } else {
            ix.accounts.swap(n - 1, n - 2);
        }
        let before = AccountSnapshot::capture(&p.env, &[ix.clone()], &p.node_pdas());
        let failed = p.env.send_err(&[ix], &[&reporter]);
        assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);
        before.assert_unchanged(&p.env);
    }
    p.assert_solvent();
}

#[test]
fn late_slashing_bounty_cpi_failure_restores_victims_registry_case_and_custody() {
    let (mut p, _, _) = setup();
    let reporter = p.reporter();
    let reporter_usdc = pda::ata(&reporter.pubkey(), &p.usdc_mint);
    freeze_token_account(&mut p, &reporter_usdc);
    let evidence = equivocation(&p, 82);
    let case = pda::punishment_case(&evidence.hash());
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &p.node_pdas());
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(1, InstructionError::Custom(17))
    );
    before.assert_unchanged(&p.env);
    assert!(!p.env.account_exists(&case));
    p.assert_solvent();
}

#[test]
fn an_invalid_raw_value_and_zero_source_leave_no_feed_or_rent_charge() {
    let (mut p, _, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let mut args = signed.submit_args();
    args.raw_value = Some(vec![9; 256]);
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidRawValue);
    before.assert_unchanged(&p.env);
    let zero = p.sign_canonical(p.live_duty(3).source_id([0; 32]));
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &zero);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidSourceId);
    before.assert_unchanged(&p.env);
}

#[test]
fn raw_values_at_supported_length_boundaries_reach_the_feed() {
    let (mut p, _, submitter) = setup();
    for length in [0usize, 1, 32, 256] {
        let raw = vec![41; length];
        let value = solana_keccak_hasher::hash(&raw).to_bytes();
        let mut source = [44; 32];
        source[..8].copy_from_slice(&(length as u64).to_le_bytes());
        let signed = p.sign_canonical(p.live_duty(3).source_id(source).value(value));
        let mut args = signed.submit_args();
        args.raw_value = Some(raw);
        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
        p.env.send_ok(&[ix], &[&submitter]);
        let feed: Feed = p.env.get(&pda::feed(&source, 3, &submitter.pubkey()));
        assert_eq!(feed.value, value);
        assert_eq!(feed.value_kind, molpha::state::FeedValueKind::Hash);
    }
}

#[test]
fn feed_quorum_mismatch_is_a_defense_in_depth_rejection_with_full_rollback() {
    let (mut p, _, submitter) = setup();
    let first = p.sign_canonical(p.live_duty(3).source_id([45; 32]));
    p.env.send_ok(
        &[p.submit_attestation_ix(&submitter.pubkey(), &first)],
        &[&submitter],
    );
    let feed_key = pda::feed(&first.duty.source_id, 3, &submitter.pubkey());
    let mut feed: Feed = p.env.get(&feed_key);
    // Defense-in-depth corrupted state: the PDA seed binds the external quorum to 3, while the
    // stored field is changed to exercise Feed::check_newer's independent guard.
    feed.signatures_required = 4;
    let mut data = Vec::with_capacity(Feed::SPACE);
    feed.try_serialize(&mut data).unwrap();
    data.resize(Feed::SPACE, 0);
    p.env.overwrite_account(&feed_key, data);

    let newer = p.sign_canonical(
        p.live_duty(3)
            .source_id(first.duty.source_id)
            .at(first.duty.timestamp + 1),
    );
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &newer);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignaturesRequiredMismatch);
    before.assert_unchanged(&p.env);
}

#[test]
fn failed_ticket_verification_rolls_back_roster_and_marker_creation() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3)).with_tampered_signature();
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
    before.assert_unchanged(&p.env);
}

#[test]
fn ticket_rejects_an_instruction_sysvar_substitution_without_creating_a_marker() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3).source_id([56; 32]));
    let mut ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    account_meta_mut(&mut ixs[1], &pda::INSTRUCTIONS_SYSVAR_ID).pubkey =
        anchor_lang::system_program::ID;
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidInstructionsSysvar);
    before.assert_unchanged(&p.env);
}

#[test]
fn ticket_outside_selection_is_rejected_with_full_rollback() {
    let (mut p, gateway, submitter) = setup();
    p.register_nodes("ticket-outsiders", 4);
    let duty = p.live_duty(3);
    let selected = p.selection_bits(&duty);
    let outsider = (0..8).find(|bit| !selected.contains(bit)).unwrap();
    let signed = p.sign_duty_with_bits(duty, &[selected[0], selected[1], outsider], None);
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
    before.assert_unchanged(&p.env);
}

#[test]
fn a_ticket_cannot_replay_through_another_gateway_or_submitter() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &signed);
    let other_gateway = p.register_gateway();
    let other_submitter = p.env.keypair();
    let ixs = p.ticket_ixs(&other_submitter.pubkey(), &other_gateway, &signed);
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&other_submitter]);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(2, InstructionError::Custom(0))
    );
    before.assert_unchanged(&p.env);
}

#[test]
fn closing_a_marker_never_reopens_its_ticket_window() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &signed);
    let marker = pda::ticket_marker(&ticket_id(&signed.duty.payload()));
    let deadline = p.env.epoch(0).ticket_deadline();
    p.env.warp_to(deadline);
    p.env.send_ok(
        &[p.close_ticket_marker_ix(&marker, &submitter.pubkey())],
        &[],
    );
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::TicketWindowClosed);
    before.assert_unchanged(&p.env);
    assert!(!p.env.account_exists(&marker));
}

#[test]
fn expired_subscriptions_can_remove_delegates_but_cannot_add_them() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);
    let delegate = p.env.keypair().pubkey();
    p.add_delegate(&owner, &delegate, 1);
    let sub: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    p.env.warp_to(sub.valid_until);
    p.env.send_ok(
        &[p.remove_delegate_ix(&owner.pubkey(), &delegate)],
        &[&owner],
    );
    let ix = p.add_delegate_ix(
        &owner.pubkey(),
        &AddDelegateArgs {
            delegate,
            max_data_requests: 1,
        },
    );
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::SubscriptionNotActive);
    before.assert_unchanged(&p.env);
    p.env.send_ok(
        &[p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic)],
        &[&owner],
    );
    p.add_delegate(&owner, &delegate, 2);
    let after: Subscription = p.env.get(&pda::subscription(&owner.pubkey()));
    assert_eq!(after.delegate_count, 1);
    assert_eq!(
        after.valid_until,
        sub.valid_until + Subscription::SECONDS_PER_MONTH
    );
}

#[test]
fn extension_overflow_after_token_transfer_restores_both_accounts() {
    let mut p = Protocol::start();
    p.add_default_plan(PlanType::Basic);
    let owner = p.subscribe(PlanType::Basic);
    let key = pda::subscription(&owner.pubkey());
    // Constructed arithmetic-defense state, not an assertion that a subscriber can create it.
    let mut data = p.env.account_data(&key);
    data[41..49].copy_from_slice(&i64::MAX.to_le_bytes());
    p.env.overwrite_account(&key, data);
    let ix = p.extend_subscription_ix(&owner.pubkey(), PlanType::Basic);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidPlanParams);
    before.assert_unchanged(&p.env);
}

#[test]
fn neither_member_account_is_rejected_without_endpoint_mutation() {
    let mut p = Protocol::start();
    let owner = p.env.keypair();
    let ix = p.update_endpoint_raw_ix(
        &owner.pubkey(),
        None,
        None,
        MemberEndpoint::Node {
            ip: [10, 0, 0, 1],
            port: 80,
        },
    );
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::ExactlyOneMember);
    before.assert_unchanged(&p.env);
}

#[test]
fn registry_snapshot_content_survives_hole_reuse_and_key_rotation() {
    let (mut p, _, _) = setup();
    let version = p.current_version();
    let before = p.env.account_data(&pda::registry(version));
    let owner = p.nodes[1].owner.insecure_clone();
    p.env
        .send_ok(&[p.deactivate_node_ix(1, &owner.pubkey())], &[&owner]);
    p.register_nodes("hole-replacement", 1);
    let replacement = p.nodes.len() - 1;
    assert_eq!(p.nodes[replacement].index, 1);
    let key = crypto::node_keys("hole-rotation", 1).remove(0);
    p.rotate_node_key(replacement, key);
    let after = p.env.account_data(&pda::registry(version));
    // Only grace_active_until, bytes 8..16, may change when a current version is retired.
    assert_eq!(&before[..8], &after[..8]);
    assert_eq!(&before[16..], &after[16..]);
    p.assert_solvent();
}

#[test]
fn solvency_includes_epochs_beyond_sixty_four() {
    let mut p = Protocol::start();
    for epoch in 0..65 {
        p.finalize_epoch(epoch);
    }
    assert!(p.live_epochs().contains(&66));
    p.env.warp_to(p.env.epoch(66).start);
    p.earn_income(101);
    assert_eq!(p.env.epoch(66).pool, 96);
    p.assert_solvent();
}

#[test]
fn close_epoch_pool_overflow_keeps_the_source_epoch_and_rent_intact() {
    let mut p = Protocol::start();
    p.earn_income(101);
    p.finalize_epoch(0);
    let source = p.env.epoch(0);
    assert!(source.pool > 0 && source.weighted_count == 0);
    p.env.warp_to(Protocol::close_deadline(&source));
    // Defense-in-depth corrupted state: normal liability accounting prevents a live epoch pool
    // from reaching u64::MAX while another epoch still has a residual.
    p.env.overwrite_epoch(2, |epoch| epoch.pool = u64::MAX);
    let ix = p.close_epoch_ix(0, 2);
    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
    let failed = p.env.send_err(&[ix], &[]);
    assert_molpha_error(&failed, MolphaError::RewardOverflow);
    before.assert_unchanged(&p.env);
}

#[test]
fn membership_churn_fills_the_roster_without_losing_known_nodes() {
    let (mut p, gateway, submitter) = setup();
    let first = p.sign_canonical(p.live_duty(3));
    p.submit_ticket(&submitter, &gateway, &first);
    let mut active = [0usize, 1, 2, 3];
    let keys = crypto::node_keys("roster-churn", 253);
    let mut final_round = None;
    for (step, key) in keys.into_iter().enumerate() {
        let lane = step % 4;
        let owner = p.nodes[active[lane]].owner.insecure_clone();
        p.env.send_ok(
            &[p.deactivate_node_ix(active[lane], &owner.pubkey())],
            &[&owner],
        );
        p.register_node_with_key(key);
        active[lane] = p.nodes.len() - 1;
        let mut source = [101; 32];
        source[..8].copy_from_slice(&(step as u64).to_le_bytes());
        let signed = p.sign_canonical(p.live_duty(3).source_id(source));
        if step == 252 {
            final_round = Some(signed);
        } else {
            p.submit_ticket(&submitter, &gateway, &signed);
        }
    }
    assert_eq!(p.env.epoch(0).roster_len, 256);
    let signed = final_round.unwrap();
    let mut hints = p.roster_slots(0, &signed.duty);
    let known = hints.iter().position(|slot| *slot < 256).unwrap();
    hints[known] = 256;
    let ix = p.submit_ticket_raw_ix(
        &submitter.pubkey(),
        &gateway.pubkey(),
        0,
        signed.attestation(),
        hints,
        signed.coalition_key,
    );
    let ixs = [p.dispatch_ix(&gateway, &signed.duty), ix];
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidRosterSlot);
    before.assert_unchanged(&p.env);
    let meta = p.submit_ticket(&submitter, &gateway, &signed);
    assert_eq!(only_event::<TicketRosterOverflow>(&meta).uncounted, 1);
    assert_eq!(p.env.epoch(0).roster_len, 256);
    p.assert_solvent();
}
