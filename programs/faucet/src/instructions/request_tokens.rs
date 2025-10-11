use crate::error::FaucetError;
use crate::state::{FaucetConfig, UserCooldown};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{mint_to, Mint, MintTo, TokenAccount, TokenInterface},
};

pub fn handler(ctx: Context<RequestTokens>) -> Result<()> {
    let faucet_config = &ctx.accounts.faucet_config;
    let clock = Clock::get()?;

    // Check if faucet is active
    require!(faucet_config.is_active, FaucetError::FaucetInactive);

    // Check cooldown
    let user_cooldown_account = &ctx.accounts.user_cooldown;
    if user_cooldown_account.last_request > 0 {
        let time_since_last_request = clock.unix_timestamp - user_cooldown_account.last_request;
        require!(
            time_since_last_request >= faucet_config.cooldown_seconds as i64,
            FaucetError::CooldownActive
        );
    }

    // Mint tokens directly to user
    let mint_key = ctx.accounts.token_mint.key();
    let seeds = &[
        FaucetConfig::SEED_PREFIX,
        mint_key.as_ref(),
        &[faucet_config.bump],
    ];
    let signer_seeds = &[&seeds[..]];

    let cpi_accounts = MintTo {
        mint: ctx.accounts.token_mint.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        authority: ctx.accounts.faucet_config.to_account_info(),
    };

    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);

    mint_to(cpi_context, faucet_config.amount_per_request)?;

    // Update user cooldown
    let user_cooldown = &mut ctx.accounts.user_cooldown;
    user_cooldown.user = ctx.accounts.user.key();
    user_cooldown.last_request = clock.unix_timestamp;
    user_cooldown.bump = ctx.bumps.user_cooldown;

    msg!(
        "Distributed {} tokens to user: {}",
        faucet_config.amount_per_request,
        ctx.accounts.user.key()
    );

    Ok(())
}

#[derive(Accounts)]
pub struct RequestTokens<'info> {
    /// Faucet configuration PDA
    #[account(
        seeds = [FaucetConfig::SEED_PREFIX, token_mint.key().as_ref()],
        bump = faucet_config.bump,
    )]
    pub faucet_config: Account<'info, FaucetConfig>,

    /// User cooldown tracking
    #[account(
        init_if_needed,
        payer = user,
        space = UserCooldown::SPACE,
        seeds = [UserCooldown::SEED_PREFIX, user.key().as_ref(), token_mint.key().as_ref()],
        bump
    )]
    pub user_cooldown: Account<'info, UserCooldown>,

    /// The SPL token mint
    #[account(mut)]
    pub token_mint: InterfaceAccount<'info, Mint>,

    /// User's token account (destination)
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = token_mint,
        associated_token::authority = user,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    /// User requesting tokens
    #[account(mut)]
    pub user: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}
