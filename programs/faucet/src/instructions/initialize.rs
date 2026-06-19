use crate::state::FaucetConfig;
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenInterface};

pub(crate) fn handler(
    ctx: Context<Initialize>,
    amount_per_request: u64,
    cooldown_seconds: u64,
    token_decimals: u8,
    token_name: String,
    token_symbol: String,
) -> Result<()> {
    let faucet_config = &mut ctx.accounts.faucet_config;

    faucet_config.authority = ctx.accounts.authority.key();
    faucet_config.token_mint = ctx.accounts.token_mint.key();
    faucet_config.amount_per_request = amount_per_request;
    faucet_config.cooldown_seconds = cooldown_seconds;
    faucet_config.is_active = true;
    faucet_config.token_decimals = token_decimals;
    faucet_config.bump = ctx.bumps.faucet_config;

    msg!(
        "Faucet initialized with mint: {}, amount per request: {}, cooldown: {}s, decimals: {}",
        faucet_config.token_mint,
        amount_per_request,
        cooldown_seconds,
        token_decimals
    );
    msg!("Token: {} ({})", token_name, token_symbol);

    Ok(())
}

#[derive(Accounts)]
#[instruction(amount_per_request: u64, cooldown_seconds: u64, token_decimals: u8)]
pub struct Initialize<'info> {
    /// Faucet configuration PDA
    #[account(
        init,
        payer = authority,
        space = FaucetConfig::SPACE,
        seeds = [FaucetConfig::SEED_PREFIX, token_mint.key().as_ref()],
        bump
    )]
    pub faucet_config: Account<'info, FaucetConfig>,

    /// The SPL token mint that this faucet will create and manage
    #[account(
        init,
        payer = authority,
        mint::decimals = token_decimals,
        mint::authority = faucet_config,
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,

    /// Authority that can manage the faucet
    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub rent: Sysvar<'info, Rent>,
}
