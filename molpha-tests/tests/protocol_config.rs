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
        max_settlement_delay_seconds: 512,
        challenger_bounty_bps: 1_250,
        x402_round_base: 2_000,
        min_signers: 5,
        epoch_len_seconds: 43_200,
        ticket_grace_seconds: 3_600,
        target_tickets_per_epoch: 4_000,
        min_availability_bps: 7_000,
        min_availability_samples: 25,
        protocol_fee_bps: 1_500,
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
        config.max_settlement_delay_seconds,
        args.max_settlement_delay_seconds
    );
    assert_eq!(config.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(config.epoch_len_seconds, args.epoch_len_seconds);
    assert_eq!(config.ticket_grace_seconds, args.ticket_grace_seconds);
    assert_eq!(
        config.target_tickets_per_epoch,
        args.target_tickets_per_epoch
    );
    assert_eq!(config.min_availability_bps, args.min_availability_bps);
    assert_eq!(
        config.min_availability_samples,
        args.min_availability_samples
    );
    assert_eq!(config.protocol_fee_bps, args.protocol_fee_bps);
    assert_eq!(config.pool_liability, 0);
    assert_eq!(config.protocol_reserved, 0);
    assert_eq!(config.minimum_node_deposit, args.minimum_node_deposit);
    assert_eq!(
        config.withdrawal_cooldown_slots,
        args.withdrawal_cooldown_slots
    );
    assert_eq!(config.x402_round_base, args.x402_round_base);
    assert_eq!(config.min_signers, args.min_signers);
    assert_eq!(
        config.reward_liability, 0,
        "a fresh protocol owes nobody anything"
    );

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

    // Genesis epochs: chained windows, current parameters, every round sampled.
    let now = b.env.unix_timestamp();
    let len = args.epoch_len_seconds as i64;
    for (index, start) in [(0u64, now), (1u64, now + len)] {
        let epoch = b.env.epoch(index);
        assert_eq!(epoch.epoch, index);
        assert_eq!((epoch.start, epoch.end), (start, start + len));
        assert_eq!(epoch.ticket_grace, args.ticket_grace_seconds as i64);
        assert_eq!(epoch.tau, u64::MAX);
        assert_eq!(epoch.min_availability_bps, args.min_availability_bps);
        assert_eq!(epoch.min_samples, args.min_availability_samples);
        assert_eq!(epoch.rent_payer, b.authority.pubkey().to_bytes());
        assert_eq!((epoch.pool, epoch.tickets, epoch.roster_len), (0, 0, 0));
        assert_eq!(epoch.finalized, 0);
    }
    assert!(!b.env.account_exists(&pda::epoch(2)));

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
        after.max_settlement_delay_seconds,
        args.max_settlement_delay_seconds
    );
    assert_eq!(after.challenger_bounty_bps, args.challenger_bounty_bps);
    assert_eq!(after.epoch_len_seconds, args.epoch_len_seconds);
    assert_eq!(after.ticket_grace_seconds, args.ticket_grace_seconds);
    assert_eq!(
        after.target_tickets_per_epoch,
        args.target_tickets_per_epoch
    );
    assert_eq!(after.min_availability_bps, args.min_availability_bps);
    assert_eq!(
        after.min_availability_samples,
        args.min_availability_samples
    );
    assert_eq!(after.protocol_fee_bps, args.protocol_fee_bps);
    assert_eq!(after.x402_round_base, args.x402_round_base);
    assert_eq!(after.min_signers, args.min_signers);
    assert_eq!(
        after.reward_liability, before.reward_liability,
        "retuning knobs must not rewrite what the protocol owes"
    );
    assert_eq!(after.pool_liability, before.pool_liability);
    assert_eq!(after.protocol_reserved, before.protocol_reserved);

    // `update_config` must not be a back door to the authority or the settlement mint.
    assert_eq!(after.authority, before.authority);
    assert_eq!(after.usdc_mint, before.usdc_mint);
    assert_eq!(after.bump, before.bump);

    let event: ProtocolConfigUpdated = only_event(&meta);
    assert_eq!(event.min_signers, args.min_signers);
    assert_eq!(event.reward_per_signature, args.reward_per_signature);
    assert_eq!(event.epoch_len_seconds, args.epoch_len_seconds);
    assert_eq!(event.min_availability_bps, args.min_availability_bps);
    assert_eq!(event.protocol_fee_bps, args.protocol_fee_bps);
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
fn initialize_rejects_a_zero_max_settlement_delay() {
    let mut b = bare();
    let args = molpha::instructions::InitializeArgs {
        max_settlement_delay_seconds: 0,
        ..default_initialize_args(b.usdc_mint)
    };
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMaxReceiptDelay);
}

#[test]
fn update_config_rejects_a_zero_max_settlement_delay() {
    let mut p = Protocol::start();
    let args = UpdateConfigArgs {
        max_settlement_delay_seconds: 0,
        ..tuned_update_args()
    };
    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidMaxReceiptDelay);
}

// ---------------------------------------------------------------------------
// challenger_bounty_bps ceiling
// ---------------------------------------------------------------------------

/// A bounty above 100% would pay a reporter more than the punishment recovered, which the
/// treasury could only cover out of other nodes' money.
#[test]
fn initialize_rejects_a_challenger_bounty_over_one_hundred_percent() {
    let mut b = bare();
    let mut args = default_initialize_args(b.usdc_mint);
    args.challenger_bounty_bps = 10_001;
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    let failed = b.env.send_err(&[ix], &[&b.authority]);
    assert_molpha_error(&failed, MolphaError::InvalidChallengerBounty);
}

#[test]
fn initialize_accepts_a_challenger_bounty_of_exactly_one_hundred_percent() {
    let mut b = bare();
    let mut args = default_initialize_args(b.usdc_mint);
    args.challenger_bounty_bps = 10_000;
    let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
    b.env.send_ok(&[ix], &[&b.authority]);
    let config: ProtocolConfig = b.env.get(&pda::protocol_config());
    assert_eq!(config.challenger_bounty_bps, 10_000);
}

#[test]
fn update_config_rejects_a_challenger_bounty_over_one_hundred_percent() {
    let mut p = Protocol::start();
    let mut args = tuned_update_args();
    args.challenger_bounty_bps = 10_001;
    let authority = p.authority.insecure_clone();
    let ix = update_config_ix(&authority.pubkey(), &args);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidChallengerBounty);

    // The rejected value never lands.
    let config: ProtocolConfig = p.env.get(&pda::protocol_config());
    assert_eq!(config.challenger_bounty_bps, p.args.challenger_bounty_bps);
}

// Run each invalid configuration through both public instructions and assert atomicity.
macro_rules! rejects_config {
    ($name:ident, $field:ident, $value:expr, $error:ident) => {
        #[test]
        fn $name() {
            let mut b = bare();
            let mut args = default_initialize_args(b.usdc_mint);
            args.$field = $value;
            let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
            let failed = b.env.send_err(&[ix], &[&b.authority]);
            assert_molpha_error(&failed, MolphaError::$error);
            // A failed initialization must leave the PDA available for a valid retry.
            let ix = initialize_ix(
                &b.authority.pubkey(),
                &b.usdc_mint,
                &default_initialize_args(b.usdc_mint),
            );
            b.env.send_ok(&[ix], &[&b.authority]);

            let mut p = Protocol::start();
            let before = p.env.account_data(&pda::protocol_config());
            let mut args = tuned_update_args();
            args.$field = $value;
            let authority = p.authority.insecure_clone();
            let ix = update_config_ix(&authority.pubkey(), &args);
            let failed = p.env.send_err(&[ix], &[&authority]);
            assert_molpha_error(&failed, MolphaError::$error);
            assert_eq!(p.env.account_data(&pda::protocol_config()), before);
        }
    };
}

rejects_config!(
    short_epoch,
    epoch_len_seconds,
    molpha::state::MIN_EPOCH_LEN_SECONDS - 1,
    InvalidEpochLength
);
rejects_config!(
    long_epoch,
    epoch_len_seconds,
    molpha::state::MAX_EPOCH_LEN_SECONDS + 1,
    InvalidEpochLength
);
rejects_config!(zero_grace, ticket_grace_seconds, 0, InvalidTicketGrace);
rejects_config!(
    grace_as_long_as_the_epoch,
    ticket_grace_seconds,
    86_400,
    InvalidTicketGrace
);
rejects_config!(
    zero_ticket_target,
    target_tickets_per_epoch,
    0,
    InvalidTicketTarget
);
rejects_config!(
    ticket_target_above_cap,
    target_tickets_per_epoch,
    molpha::state::MAX_TICKETS_PER_EPOCH / 8 + 1,
    InvalidTicketTarget
);
rejects_config!(
    excessive_availability_floor,
    min_availability_bps,
    10_001,
    InvalidAvailabilityFloor
);
rejects_config!(
    zero_availability_samples,
    min_availability_samples,
    0,
    InvalidAvailabilityFloor
);
rejects_config!(
    excessive_protocol_fee,
    protocol_fee_bps,
    10_001,
    InvalidProtocolFee
);
rejects_config!(
    zero_deposit,
    minimum_node_deposit,
    0,
    InvalidMinimumNodeDeposit
);
rejects_config!(
    unrepresentable_delay,
    max_settlement_delay_seconds,
    u64::MAX,
    InvalidMaxReceiptDelay
);
rejects_config!(
    delay_above_cap,
    max_settlement_delay_seconds,
    molpha::state::MAX_SETTLEMENT_DELAY_CAP_SECONDS + 1,
    InvalidMaxReceiptDelay
);
rejects_config!(
    overflowing_timestamp,
    max_settlement_delay_seconds,
    i64::MAX as u64,
    InvalidMaxReceiptDelay
);
rejects_config!(
    overflowing_cooldown,
    withdrawal_cooldown_slots,
    u64::MAX,
    InvalidWithdrawalCooldown
);
rejects_config!(
    overflowing_rewards,
    reward_per_signature,
    u64::MAX,
    InvalidRoundPrice
);
rejects_config!(
    overflowing_price,
    x402_round_base,
    u64::MAX,
    InvalidRoundPrice
);

#[test]
fn initialize_rejects_invalid_redundancy_without_panicking() {
    for redundancy_buffer in [0, 9, u8::MAX] {
        let mut b = bare();
        let mut args = default_initialize_args(b.usdc_mint);
        args.redundancy_buffer = redundancy_buffer;
        let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
        let failed = b.env.send_err(&[ix], &[&b.authority]);
        assert_molpha_error(&failed, MolphaError::InvalidRedundancyBuffer);
    }
}

#[test]
fn config_accepts_inclusive_boundaries_and_zero_optional_economics() {
    for low in [true, false] {
        let mut b = bare();
        let mut args = default_initialize_args(b.usdc_mint);
        args.epoch_len_seconds = if low {
            molpha::state::MIN_EPOCH_LEN_SECONDS
        } else {
            molpha::state::MAX_EPOCH_LEN_SECONDS
        };
        args.ticket_grace_seconds = if low { 1 } else { args.epoch_len_seconds - 1 };
        args.target_tickets_per_epoch = if low {
            1
        } else {
            molpha::state::MAX_TICKETS_PER_EPOCH / 8
        };
        args.min_availability_bps = if low { 0 } else { 10_000 };
        args.min_availability_samples = if low { 1 } else { u16::MAX };
        args.protocol_fee_bps = if low { 0 } else { 10_000 };
        args.redundancy_buffer = if low { 1 } else { 8 };
        args.challenger_bounty_bps = 0;
        args.reward_per_signature = 0;
        args.x402_round_base = 0;
        let ix = initialize_ix(&b.authority.pubkey(), &b.usdc_mint, &args);
        b.env.send_ok(&[ix], &[&b.authority]);
        let mut update = tuned_update_args();
        update.protocol_fee_bps = if low { 10_000 } else { 0 };
        update.challenger_bounty_bps = 10_000;
        update.reward_per_signature = 0;
        update.x402_round_base = u64::MAX;
        let ix = update_config_ix(&b.authority.pubkey(), &update);
        b.env.send_ok(&[ix], &[&b.authority]);
    }
}
