#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use state::*;

declare_id!("72vzGSKhPohoatawLW2W8mugeWbZRCdPZ3M4dVNDvkRz");

#[program]
pub mod molpha {
    use super::*;

    // Protocol initialization
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        instructions::initialize(ctx)
    }

    // Node management
    pub fn register_node(
        ctx: Context<RegisterNode>,
        node_pubkey: Pubkey,
        evm_pubkey: Vec<u8>,
    ) -> Result<()> {
        instructions::register_node(ctx, node_pubkey, evm_pubkey)
    }

    // Feed management functions (from molpha-feed)
    pub fn create_feed(
        ctx: Context<CreateFeed>,
        params: CreateFeedParams,
        subscription_duration_seconds: u64,
    ) -> Result<()> {
        instructions::create_feed(ctx, params, subscription_duration_seconds)
    }

    pub fn extend_subscription(
        ctx: Context<ExtendSubscription>,
        additional_duration_seconds: u64,
    ) -> Result<()> {
        instructions::extend_subscription(ctx, additional_duration_seconds)
    }

    pub fn update_feed_config(
        ctx: Context<UpdateFeedConfig>,
        params: UpdateFeedConfigParams,
    ) -> Result<()> {
        instructions::update_feed_config(ctx, params)
    }

    pub fn publish_answer(ctx: Context<PublishAnswer>, answer: Answer) -> Result<()> {
        instructions::publish_answer(ctx, answer)
    }

    // Data source management functions
    pub fn create_data_source(ctx: Context<CreateDataSource>, data: DataSourceInfo) -> Result<()> {
        instructions::create_data_source(ctx, data)
    }

    pub fn update_data_source(
        ctx: Context<UpdateDataSource>,
        metadata_hash: [u8; 32],
    ) -> Result<()> {
        instructions::update_data_source(ctx, metadata_hash)
    }

    // Protocol configuration management
    pub fn update_protocol_config(
        ctx: Context<UpdateProtocolConfig>,
        params: UpdateProtocolConfigParams,
    ) -> Result<()> {
        instructions::update_protocol_config(ctx, params)
    }
}
