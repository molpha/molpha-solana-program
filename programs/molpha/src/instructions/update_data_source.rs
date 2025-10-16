use anchor_lang::prelude::*;

use crate::state::DataSource;

pub fn update_data_source(ctx: Context<UpdateDataSource>, metadata_hash: [u8; 32]) -> Result<()> {
    let data_source = &mut ctx.accounts.data_source;
    data_source.metadata_hash = metadata_hash;
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
