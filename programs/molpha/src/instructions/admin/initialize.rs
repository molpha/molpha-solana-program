use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

use crate::{
    error::MolphaError,
    events::{EpochCreated, ProtocolInitialized},
    state::{epoch::TAU_MIN, EpochState, ProtocolConfig, Registry, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeArgs {
    pub withdrawal_cooldown_slots: u64,
    pub redundancy_buffer: u8,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_settlement_delay_seconds: u64,
    pub challenger_bounty_bps: u16,
    pub usdc_mint: Pubkey,
    pub x402_round_base: u64,
    pub min_signers: u8,
    pub epoch_len_seconds: u64,
    pub ticket_grace_seconds: u64,
    pub target_tickets_per_epoch: u32,
    pub min_availability_bps: u16,
    pub min_availability_samples: u16,
    pub protocol_fee_bps: u16,
    /// Ticket threshold of the two genesis epochs: a round is a ticket when the 64-bit prefix of its
    /// ticket hash is below it, so the sampling probability is `initial_tau / 2^64`. Size it as
    /// `2^64 * target_tickets_per_epoch / expected_rounds_per_epoch`. `u64::MAX` samples every round
    /// (every round then costs a ticket transaction until the controller, which moves the
    /// threshold at most 8x per epoch with a two-epoch lag, brings it down). Must be at least
    /// `TAU_MIN`.
    pub initial_tau: u64,
}

pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
    let clock = Clock::get()?;

    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.authority = ctx.accounts.authority.key();
    protocol_config.usdc_mint = args.usdc_mint;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_settlement_delay_seconds = args.max_settlement_delay_seconds;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_slots = args.withdrawal_cooldown_slots;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.min_signers = args.min_signers;
    protocol_config.epoch_len_seconds = args.epoch_len_seconds;
    protocol_config.ticket_grace_seconds = args.ticket_grace_seconds;
    protocol_config.target_tickets_per_epoch = args.target_tickets_per_epoch;
    protocol_config.min_availability_bps = args.min_availability_bps;
    protocol_config.min_availability_samples = args.min_availability_samples;
    protocol_config.protocol_fee_bps = args.protocol_fee_bps;
    protocol_config.validate()?;
    protocol_config.bump = ctx.bumps.protocol_config;
    require!(args.initial_tau >= TAU_MIN, MolphaError::InvalidInitialTau);

    require!(
        (1..=8).contains(&args.redundancy_buffer),
        MolphaError::InvalidRedundancyBuffer
    );

    ctx.accounts.registry_state.init(ctx.bumps.registry_state);
    ctx.accounts.registry.load_init()?.init_genesis(
        args.redundancy_buffer,
        clock.unix_timestamp,
        ctx.bumps.registry,
    );

    // Genesis epochs: every later epoch is created two ahead by `finalize_epoch`, and starts
    // where its predecessor ends. Both genesis epochs use `initial_tau`; the controller takes over
    // from epoch 2, which `finalize_epoch` creates from epoch 0's observed ticket count.
    let config = &ctx.accounts.protocol_config;
    let len = config.epoch_len_seconds as i64;
    let grace = config.ticket_grace_seconds as i64;
    let authority = ctx.accounts.authority.key();
    let mut start = clock.unix_timestamp;
    for (epoch, loader, bump) in [
        (0u64, &ctx.accounts.epoch0, ctx.bumps.epoch0),
        (1u64, &ctx.accounts.epoch1, ctx.bumps.epoch1),
    ] {
        let end = start
            .checked_add(len)
            .ok_or(MolphaError::InvalidEpochLength)?;
        loader.load_init()?.init(
            epoch,
            start,
            end,
            grace,
            args.initial_tau,
            config.min_availability_bps,
            config.min_availability_samples,
            config.min_signers,
            authority,
            bump,
        );
        emit!(EpochCreated {
            epoch,
            start,
            end,
            ticket_grace: grace,
            tau: args.initial_tau,
        });
        start = end;
    }

    emit!(ProtocolInitialized {
        protocol_config: ctx.accounts.protocol_config.key(),
        registry_state: ctx.accounts.registry_state.key(),
        registry: ctx.accounts.registry.key(),
        treasury: ctx.accounts.treasury.key(),
        node_deposit_vault: ctx.accounts.node_deposit_vault.key(),
        authority: ctx.accounts.authority.key(),
        usdc_mint: ctx.accounts.usdc_mint.key(),
        initialized_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: InitializeArgs)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = ProtocolConfig::SPACE,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        init,
        payer = authority,
        space = RegistryState::SPACE,
        seeds = [RegistryState::SEED_PREFIX],
        bump
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        init,
        payer = authority,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, 0u32.to_le_bytes().as_ref()],
        bump
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        constraint = usdc_mint.key() == args.usdc_mint @ MolphaError::InvalidMint
    )]
    pub usdc_mint: Box<Account<'info, Mint>>,

    // `init_if_needed`: both vault addresses are predictable, and anyone may create an ATA for any
    // owner, so a plain `init` could be front-run into a permanent failure.
    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    /// One vault for every node's security deposit, held apart from protocol revenue.
    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        init,
        payer = authority,
        space = EpochState::SPACE,
        seeds = [EpochState::SEED_PREFIX, 0u64.to_le_bytes().as_ref()],
        bump
    )]
    pub epoch0: AccountLoader<'info, EpochState>,

    #[account(
        init,
        payer = authority,
        space = EpochState::SPACE,
        seeds = [EpochState::SEED_PREFIX, 1u64.to_le_bytes().as_ref()],
        bump
    )]
    pub epoch1: AccountLoader<'info, EpochState>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
