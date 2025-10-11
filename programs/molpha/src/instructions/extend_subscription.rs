use crate::error::FeedError;
use crate::events::SubscriptionExtended;
use crate::state::{Feed, ProtocolConfig};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

pub fn extend_subscription(
    ctx: Context<ExtendSubscription>,
    additional_duration_seconds: u64,
) -> Result<()> {
    let feed = &mut ctx.accounts.feed;

    require!(
        additional_duration_seconds >= 86400, // At least 1 day
        FeedError::MinimumExtensionTime
    );

    let new_due_datetime = if feed.subscription_due_time > Clock::get()?.unix_timestamp {
        feed.subscription_due_time + additional_duration_seconds as i64
    } else {
        Clock::get()?.unix_timestamp + additional_duration_seconds as i64
    };

    // Calculate cost for extension (like SubscriptionRegistry.extendSubscription)
    let extension_cost =
        (feed.price_per_second_scaled * additional_duration_seconds) / ProtocolConfig::SCALAR;

    // Extend subscription
    feed.subscription_due_time = new_due_datetime;

    // Transfer tokens from user to program token account
    let decimals = ctx.accounts.underlying_token.decimals;
    let cpi_accounts = TransferChecked {
        from: ctx.accounts.user_token_account.to_account_info(),
        to: ctx.accounts.program_token_account.to_account_info(),
        authority: ctx.accounts.authority.to_account_info(),
        mint: ctx.accounts.underlying_token.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_context = CpiContext::new(cpi_program, cpi_accounts);
    transfer_checked(cpi_context, extension_cost, decimals)?;

    // Emit event
    emit!(SubscriptionExtended {
        feed: ctx.accounts.feed.key(),
        authority: ctx.accounts.authority.key(),
        additional_duration: additional_duration_seconds,
        new_due_time: new_due_datetime,
        extended_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ExtendSubscription<'info> {
    #[account(
        mut,
        has_one = authority,
    )]
    pub feed: Account<'info, Feed>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// User's associated token account to transfer tokens from
    #[account(
        mut,
        associated_token::mint = underlying_token,
        associated_token::authority = authority,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    /// Program's associated token account to receive tokens
    #[account(
        mut,
        associated_token::mint = underlying_token,
        associated_token::authority = protocol_config,
    )]
    pub program_token_account: InterfaceAccount<'info, TokenAccount>,

    /// Protocol config to get the underlying token authority
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// The underlying token mint
    pub underlying_token: InterfaceAccount<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}
