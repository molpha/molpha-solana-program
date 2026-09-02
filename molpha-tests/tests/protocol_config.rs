//! `initialize` and `update_config`.

use molpha::error::MolphaError;
use molpha::events::{ProtocolConfigUpdated, ProtocolInitialized};
use molpha::instructions::UpdateConfigArgs;
use molpha::state::{ProtocolConfig, RegistryState, MIN_SIGNERS_FLOOR};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{
    bare, default_initialize_args, initialize_ix, update_config_ix, Protocol,
};
use molpha_tests::token;
use solana_signer::Signer;

fn tuned_update_args() -> UpdateConfigArgs {
    UpdateConfigArgs {
        withdrawal_cooldown_slots: 7_200,
        minimum_node_deposit: 250_000_000,
        reward_per_signature: 11,
        max_receipt_delay_seconds: 512,
        dispute_window_slots: 4_096,
        gateway_bond_min: 900_000_000,
        challenger_bounty_bps: 1_250,
        x402_round_base: 2_000,
        min_signers: 5,
    }
}

#[test]
fn initialize_writes_config_registry_state_and_genesis_snapshot() {
    let mut b = bare();
    let args = default_initialize_args(b.usdc_mint);
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let meta = b.env.send_ok(&[ix], &[&b.authority]);

    let config: ProtocolConfig = b.env.get(&pda::protocol_config());
    assert_eq!(config.authority, b.authority.pubkey());
    assert_eq!(config.usdc_mint, b.usdc_mint);
    assert_eq!(config.reward_per_signature, args.reward_per_signature);
    assert_eq!(
        config.max_receipt_delay_seconds,
        args.max_receipt_delay_seconds
    );
    assert_eq!(config.dispute_window_slots, args.dispute_window_slots);
    assert_eq!(config.gateway_bond_min, args.gateway_bond_min);
    assert_eq!(config.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(config.minimum_node_deposit, args.minimum_node_deposit);
    assert_eq!(
        config.withdrawal_cooldown_slots,
        args.withdrawal_cooldown_slots
    );
    assert_eq!(config.x402_round_base, args.x402_round_base);
    assert_eq!(config.min_signers, args.min_signers);

    let registry_state: RegistryState = b.env.get(&pda::registry_state());
    assert_eq!(registry_state.current_version, 0);

    let genesis = b.env.registry(0);
    assert_eq!(genesis.version, 0);
    assert_eq!(genesis.node_count, 0);
    assert_eq!(genesis.redundancy_buffer, args.redundancy_buffer);
    assert!(genesis.nodes.is_empty());

    // Treasury is a protocol-config-owned ATA that starts empty.
    let treasury = pda::ata(&pda::protocol_config(), &b.usdc_mint);
    let state = token::unpack_token_account(&b.env.account_data(&treasury));
    assert_eq!(state.mint, b.usdc_mint);
    assert_eq!(state.owner, pda::protocol_config());
    assert_eq!(state.amount, 0);

    let event: ProtocolInitialized = only_event(&meta);
    assert_eq!(event.protocol_config, pda::protocol_config());
    assert_eq!(event.registry_state, pda::registry_state());
    assert_eq!(event.registry, pda::registry(0));
    assert_eq!(event.treasury, treasury);
    assert_eq!(event.authority, b.authority.pubkey());
    assert_eq!(event.usdc_mint, b.usdc_mint);
}

#[test]
fn initialize_rejects_min_signers_below_the_floor() {
    let mut b = bare();
    let args = molpha::instructions::InitializeArgs {
        min_signers: MIN_SIGNERS_FLOOR - 1,
        ..default_initialize_args(b.usdc_mint)
    };
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMinSigners);
}

#[test]
fn initialize_rejects_a_zero_withdrawal_cooldown() {
    let mut b = bare();
    let args = molpha::instructions::InitializeArgs {
        withdrawal_cooldown_slots: 0,
        ..default_initialize_args(b.usdc_mint)
    };
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidWithdrawalCooldown);
}

#[test]
fn initialize_accepts_min_signers_exactly_at_the_floor() {
    let mut b = bare();
    let args = molpha::instructions::InitializeArgs {
        min_signers: MIN_SIGNERS_FLOOR,
        ..default_initialize_args(b.usdc_mint)
    };
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    b.env.send_ok(&[ix], &[&b.authority]);

    let config: ProtocolConfig = b.env.get(&pda::protocol_config());
    assert_eq!(config.min_signers, MIN_SIGNERS_FLOOR);
}

#[test]
fn initialize_rejects_a_mint_account_that_disagrees_with_the_args() {
    let mut b = bare();
    let (other_mint, _) = b.env.create_mint(6);
    // `args.usdc_mint` names the real mint; the account passed in is a different one.
    let args = default_initialize_args(b.usdc_mint);
    let ix = initialize_ix(&b.authority.pubkey(), &other_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMint);
}

#[test]
fn initialize_rejects_a_token_2022_mint() {
    let mut b = bare();
    let mint = b.env.plant_token_2022_mint(6);
    let args = default_initialize_args(mint);
    let ix = initialize_ix(&b.authority.pubkey(), &mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    // The `Account<Mint>` owner check fires before any Molpha-specific constraint.
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountOwnedByWrongProgram,
    );
}

#[test]
fn initialize_cannot_be_run_twice() {
    let mut p = Protocol::start();
    let args = default_initialize_args(p.usdc_mint);
    let ix = initialize_ix(&p.authority.pubkey(), &p.usdc_mint, &args);
    let authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&authority]);
    // `init` on an existing account: the system program refuses to re-create it.
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "expected an 'already in use' failure, logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn update_config_rewrites_every_tunable_and_leaves_identity_alone() {
    let mut p = Protocol::start();
    let before: ProtocolConfig = p.env.get(&pda::protocol_config());
    let args = tuned_update_args();

    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let meta = p.env.send_ok(&[ix], &[&authority]);

    let after: ProtocolConfig = p.env.get(&pda::protocol_config());
    assert_eq!(
        after.withdrawal_cooldown_slots,
        args.withdrawal_cooldown_slots
    );
    assert_eq!(after.minimum_node_deposit, args.minimum_node_deposit);
    assert_eq!(after.reward_per_signature, args.reward_per_signature);
    assert_eq!(
        after.max_receipt_delay_seconds,
        args.max_receipt_delay_seconds
    );
    assert_eq!(after.dispute_window_slots, args.dispute_window_slots);
    assert_eq!(after.gateway_bond_min, args.gateway_bond_min);
    assert_eq!(after.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(after.x402_round_base, args.x402_round_base);
    assert_eq!(after.min_signers, args.min_signers);

    // `update_config` must not be a back door to the authority or the settlement mint.
    assert_eq!(after.authority, before.authority);
    assert_eq!(after.usdc_mint, before.usdc_mint);
    assert_eq!(after.bump, before.bump);

    let event: ProtocolConfigUpdated = only_event(&meta);
    assert_eq!(event.min_signers, args.min_signers);
    assert_eq!(event.reward_per_signature, args.reward_per_signature);
}

#[test]
fn update_config_rejects_a_caller_that_is_not_the_authority() {
    let mut p = Protocol::start();
    let impostor = p.env.keypair();
    let ix = update_config_ix(&impostor.pubkey(), &tuned_update_args());
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);

    let config: ProtocolConfig = p.env.get(&pda::protocol_config());
    assert_eq!(config.reward_per_signature, p.args.reward_per_signature);
}

#[test]
fn update_config_rejects_min_signers_below_the_floor() {
    let mut p = Protocol::start();
    let args = UpdateConfigArgs {
        min_signers: MIN_SIGNERS_FLOOR - 1,
        ..tuned_update_args()
    };
    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMinSigners);

    // The rejected write must not have partially landed.
    let config: ProtocolConfig = p.env.get(&pda::protocol_config());
    assert_eq!(config.min_signers, 3);
}

#[test]
fn update_config_rejects_a_zero_withdrawal_cooldown() {
    let mut p = Protocol::start();
    let args = UpdateConfigArgs {
        withdrawal_cooldown_slots: 0,
        ..tuned_update_args()
    };
    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidWithdrawalCooldown);
}

#[test]
fn initialize_rejects_a_zero_max_receipt_delay() {
    let mut b = bare();
    let args = molpha::instructions::InitializeArgs {
        max_receipt_delay_seconds: 0,
        ..default_initialize_args(b.usdc_mint)
    };
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMaxReceiptDelay);
}

#[test]
fn update_config_rejects_a_zero_max_receipt_delay() {
    let mut p = Protocol::start();
    let args = UpdateConfigArgs {
        max_receipt_delay_seconds: 0,
        ..tuned_update_args()
    };
    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMaxReceiptDelay);
}
