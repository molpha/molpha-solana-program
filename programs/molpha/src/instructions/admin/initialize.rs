use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

use crate::{
    error::MolphaError,
    events::ProtocolInitialized,
    state::{ProtocolConfig, Registry, RegistryState, MIN_SIGNERS_FLOOR},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeArgs {
    pub withdrawal_cooldown_slots: u64,
    pub redundancy_buffer: u8,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_receipt_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub usdc_mint: Pubkey,
    pub x402_round_base: u64,
    pub x402_round_per_sig: u64,
    pub min_signers: u8,
}

pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
    let clock = Clock::get()?;

    require!(
        args.withdrawal_cooldown_slots > 0,
        MolphaError::InvalidWithdrawalCooldown
    );
    require!(
        args.max_receipt_delay_seconds > 0,
        MolphaError::InvalidMaxReceiptDelay
    );
    require!(
        args.min_signers >= MIN_SIGNERS_FLOOR,
        MolphaError::InvalidMinSigners
    );

    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.authority = ctx.accounts.authority.key();
    protocol_config.usdc_mint = args.usdc_mint;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_receipt_delay_seconds = args.max_receipt_delay_seconds;
    protocol_config.dispute_window_slots = args.dispute_window_slots;
    protocol_config.gateway_bond_min = args.gateway_bond_min;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_slots = args.withdrawal_cooldown_slots;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.x402_round_per_sig = args.x402_round_per_sig;
    protocol_config.min_signers = args.min_signers;
    protocol_config.bump = ctx.bumps.protocol_config;

    ctx.accounts.registry_state.init(ctx.bumps.registry_state);
    ctx.accounts
        .registry
        .load_init()?
        .init_genesis(args.redundancy_buffer, ctx.bumps.registry);

    emit!(ProtocolInitialized {
        protocol_config: ctx.accounts.protocol_config.key(),
        registry_state: ctx.accounts.registry_state.key(),
        registry: ctx.accounts.registry.key(),
        treasury: ctx.accounts.treasury.key(),
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
    pub protocol_config: Account<'info, ProtocolConfig>,

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
    pub usdc_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
