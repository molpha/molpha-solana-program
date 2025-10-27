use crate::error::FeedError;
use crate::events::FeedConfigUpdated;
use crate::state::{Feed, FeedType, ProtocolConfig};
use crate::utils::pricing::calculate_price_per_second_scaled;
use anchor_lang::prelude::*;

pub fn update_feed_config(
    ctx: Context<UpdateFeedConfig>,
    params: UpdateFeedConfigParams,
) -> Result<()> {
    let feed = &mut ctx.accounts.feed;

    require!(
        feed.feed_type == FeedType::Personal,
        FeedError::NotSupported
    );
    require!(!params.ipfs_cid.is_empty(), FeedError::InvalidFeedConfig);
    require!(
        params.min_signatures_threshold > 0,
        FeedError::InvalidFeedConfig
    );

    let old_price_per_second_scaled = feed.price_per_second_scaled;

    feed.min_signatures_threshold = params.min_signatures_threshold;
    feed.frequency = params.frequency;
    feed.ipfs_cid = params.ipfs_cid.clone();
    feed.job_id = params.job_id;

    let price_per_second_scaled =
        calculate_price_per_second_scaled(feed, &ctx.accounts.protocol_config)?;
    let time_left = feed.subscription_due_time as u64 - Clock::get()?.unix_timestamp as u64;
    let new_due_time = Clock::get()?.unix_timestamp as u64
        + ((time_left * old_price_per_second_scaled) / price_per_second_scaled);
    feed.subscription_due_time = new_due_time as i64;
    feed.price_per_second_scaled = price_per_second_scaled;

    // Emit event
    emit!(FeedConfigUpdated {
        feed: feed.key(),
        authority: feed.authority,
        min_signatures_threshold: feed.min_signatures_threshold,
        frequency: feed.frequency,
        job_id: feed.job_id,
        price_per_second_scaled: price_per_second_scaled,
        subscription_due_time: feed.subscription_due_time,
        ipfs_cid: feed.ipfs_cid.clone(),
        updated_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateFeedConfig<'info> {
    #[account(
        mut,
        has_one = authority
    )]
    pub feed: Account<'info, Feed>,
    pub authority: Signer<'info>,
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct UpdateFeedConfigParams {
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    pub ipfs_cid: String,
    pub job_id: [u8; 32],
}
