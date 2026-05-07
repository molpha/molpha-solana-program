use anchor_lang::prelude::*;

use crate::{events::DataSourceUpdated, state::DataSource};

pub fn update_data_source(ctx: Context<UpdateDataSource>, metadata_hash: [u8; 32]) -> Result<()> {
    let data_source = &mut ctx.accounts.data_source;
    data_source.metadata_hash = metadata_hash;

    emit!(DataSourceUpdated {
        data_source: data_source.key(),
        authority: data_source.authority,
        metadata_hash: data_source.metadata_hash,
        updated_at: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateDataSource<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        has_one = authority
    )]
    pub data_source: Account<'info, DataSource>,
}
