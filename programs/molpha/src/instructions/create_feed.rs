use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

use crate::{
    error::FeedError,
    events::FeedCreated,
    state::{DataSource, DataSourceType, Feed, FeedType, ProtocolConfig, MAX_HISTORY},
    utils::{pricing::*, seeds::to_seed},
};

pub fn create_feed(
    ctx: Context<CreateFeed>,
    params: CreateFeedParams,
    subscription_duration_seconds: u64,
) -> Result<()> {
    require!(
        params.min_signatures_threshold > 0,
        FeedError::InvalidFeedConfig
    );
    require!(!params.ipfs_cid.is_empty(), FeedError::InvalidFeedConfig);
    require!(
        subscription_duration_seconds >= 86400,
        FeedError::MinimumSubscriptionTime
    ); // At least 1 day

    let data_source = &ctx.accounts.data_source;
    if data_source.data_source_type == DataSourceType::Personal {
        require!(
            data_source.authority == ctx.accounts.authority.key(),
            FeedError::InvalidDataSource
        );
    }

    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.protocol_config;
    let data_source = &mut ctx.accounts.data_source;

    // Initialize feed with basic data
    let feed = &mut ctx.accounts.feed;
    feed.name = params.name;
    feed.authority = ctx.accounts.authority.key();
    feed.feed_type = params.feed_type;
    feed.min_signatures_threshold = params.min_signatures_threshold;
    feed.frequency = params.frequency;
    feed.ipfs_cid = params.ipfs_cid;
    feed.job_id = params.job_id;
    feed.data_source = data_source.key();
    feed.answer_history = Vec::with_capacity(MAX_HISTORY);
    feed.created_at = now;
    feed.bump = ctx.bumps.feed;
    // Calculate subscription pricing (like PricingHelper.calculatePrice)
    let price_per_second_scaled = calculate_price_per_second_scaled(feed, config)?;

    // Calculate total subscription cost
    let subscription_cost =
        (price_per_second_scaled * subscription_duration_seconds) / ProtocolConfig::SCALAR;

    // Initialize subscription data
    feed.subscription_due_time = now + subscription_duration_seconds as i64;
    feed.price_per_second_scaled = price_per_second_scaled;

    // Transfer tokens from user to program token account
    let token_program = &ctx.accounts.token_program;
    let user_token_account = &ctx.accounts.user_token_account;
    let program_token_account = &ctx.accounts.program_token_account;
    let authority = &ctx.accounts.authority;

    let payment_cpi_ctx = CpiContext::new(
        token_program.to_account_info(),
        token::Transfer {
            from: user_token_account.to_account_info(),
            to: program_token_account.to_account_info(),
            authority: authority.to_account_info(),
        },
    );
    token::transfer(payment_cpi_ctx, subscription_cost)?;

    emit!(FeedCreated {
        key: feed.key(),
        authority: feed.authority,
        feed_type: feed.feed_type,
        min_signatures_threshold: feed.min_signatures_threshold,
        frequency: feed.frequency,
        ipfs_cid: feed.ipfs_cid.clone(),
        data_source: feed.data_source,
        created_at: now,
        subscription_due_time: feed.subscription_due_time,
        total_cost: subscription_cost,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(params: CreateFeedParams, subscription_duration_seconds: u64)]
pub struct CreateFeed<'info> {
    #[account(
        init,
        payer = authority,
        space = Feed::SPACE,
        seeds = [
            Feed::SEED_PREFIX,
            authority.key().as_ref(),
            &params.name_seed(),
            params.feed_type.to_seed().as_ref(),
            params.min_signatures_threshold.to_le_bytes().as_ref(),
            params.frequency.to_le_bytes().as_ref(),
            params.job_id.as_ref(),
        ],
        bump
    )]
    pub feed: Account<'info, Feed>,

    pub data_source: Account<'info, DataSource>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

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

    /// The underlying token mint
    pub underlying_token: InterfaceAccount<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct CreateFeedParams {
    pub name: String,
    pub job_id: [u8; 32],
    pub feed_type: FeedType,
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    pub ipfs_cid: String,
}

impl CreateFeedParams {
    pub fn name_seed(&self) -> [u8; 32] {
        to_seed(&self.name)
    }
}