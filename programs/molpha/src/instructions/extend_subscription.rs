use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::SubscriptionExtended,
    state::{Plan, ProtocolConfig, Subscription},
};

pub fn extend_subscription(ctx: Context<ExtendSubscription>) -> Result<()> {
    let plan = &ctx.accounts.plan;
    let monthly_price = plan.subscription_price;
    let now = Clock::get()?.unix_timestamp;

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.owner_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        monthly_price,
    )?;

    let subscription = &mut ctx.accounts.subscription;
    subscription.prepaid_usdc = subscription
        .prepaid_usdc
        .checked_add(monthly_price)
        .ok_or(MolphaError::InvalidPlanParams)?;
    subscription.price = monthly_price;
    subscription.valid_until = if subscription.valid_until > now {
        subscription
            .valid_until
            .checked_add(Subscription::SECONDS_PER_MONTH)
            .ok_or(MolphaError::InvalidPlanParams)?
    } else {
        now.checked_add(Subscription::SECONDS_PER_MONTH)
            .ok_or(MolphaError::InvalidPlanParams)?
    };

    emit!(SubscriptionExtended {
        owner: ctx.accounts.owner.key(),
        plan_type: subscription.plan_type as u8,
        prepaid_usdc: subscription.prepaid_usdc,
        valid_until: subscription.valid_until,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ExtendSubscription<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Subscription::SEED_PREFIX, owner.key().as_ref()],
        bump = subscription.bump,
        constraint = subscription.owner == owner.key() @ MolphaError::UnauthorisedJobOwner,
    )]
    pub subscription: Account<'info, Subscription>,

    #[account(
        seeds = [Plan::SEED_PREFIX, &[subscription.plan_type as u8]],
        bump = plan.bump,
        constraint = plan.is_active @ MolphaError::PlanNotActive,
    )]
    pub plan: Account<'info, Plan>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(
        mut,
        constraint = owner_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = owner_usdc.owner == owner.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub owner_usdc: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
