#![allow(deprecated)]

pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("9kKrAJsk287onKUgcqAtYF5P8WF65uyWC9yuujxAjXMj");

#[program]
pub mod faucet {
    use super::*;

    /// Initialize a new faucet for a specific token mint
    pub fn initialize(
        ctx: Context<Initialize>,
        amount_per_request: u64,
        cooldown_seconds: u64,
        token_decimals: u8,
        token_name: String,
        token_symbol: String,
    ) -> Result<()> {
        initialize::handler(
            ctx,
            amount_per_request,
            cooldown_seconds,
            token_decimals,
            token_name,
            token_symbol,
        )
    }

    /// Request tokens from the faucet
    pub fn request_tokens(ctx: Context<RequestTokens>) -> Result<()> {
        request_tokens::handler(ctx)
    }
}
