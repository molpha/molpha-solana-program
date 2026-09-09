use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::Subscribed,
    state::{Plan, PlanType, ProtocolConfig, Subscription},
};

pub fn subscribe(ctx: Context<Subscribe>, plan_type: PlanType) -> Result<()> {
    let plan = &ctx.accounts.plan;
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
        plan.subscription_price,
    )?;

    ctx.accounts.subscription.init(ctx.accounts.owner.key(), plan, now, ctx.bumps.subscription);

    emit!(Subscribed {
        owner: ctx.accounts.owner.key(),
        plan_type: plan_type as u8,
        prepaid_usdc: plan.subscription_price,
        price_snapshot: plan.subscription_price,
        subcribed_at: now,
        valid_until: ctx.accounts.subscription.valid_until,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(plan_type: PlanType)]
pub struct Subscribe<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [Plan::SEED_PREFIX, &[plan_type as u8]],
        bump = plan.bump,
        constraint = plan.is_active @ MolphaError::PlanNotActive,
    )]
    pub plan: Account<'info, Plan>,

    #[account(
        init,
        payer = owner,
        space = Subscription::SPACE,
        seeds = [Subscription::SEED_PREFIX, owner.key().as_ref()],
        bump
    )]
    pub subscription: Account<'info, Subscription>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = owner,
        associated_token::token_program = token_program,
    )]
    pub owner_usdc: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
