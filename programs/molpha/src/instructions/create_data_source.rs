use anchor_lang::prelude::*;

use crate::{
    error::DataSourceError,
    events::DataSourceCreated,
    state::{DataSource, DataSourceInfo, DataSourceType},
    utils::hash::to_hash,
};

pub fn create_data_source(ctx: Context<CreateDataSource>, data: DataSourceInfo) -> Result<()> {
    let authority = ctx.accounts.authority.key();
    let clock = Clock::get()?;
    let data_source = &mut ctx.accounts.data_source;

    require!(
        data.data_source_type == DataSourceType::Personal
            || data.data_source_type == DataSourceType::Public,
        DataSourceError::InvalidDataSourceType
    );
    require!(
        !data.name.is_empty(),
        DataSourceError::InvalidDataSourceData
    );

    data_source.authority = authority;
    data_source.data_source_type = data.data_source_type;
    data_source.metadata_hash = data.metadata_hash;
    data_source.name_hash = to_hash(&data.name);
    data_source.created_at = clock.unix_timestamp;
    data_source.bump = ctx.bumps.data_source;

    // Emit event
    emit!(DataSourceCreated {
        id: data_source.key(),
        owner: authority,
        name: data.name.clone(),
        name_hash: data_source.name_hash,
        data_source_type: data_source.data_source_type,
        metadata_hash: data_source.metadata_hash,
        created_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(data: DataSourceInfo)]
pub struct CreateDataSource<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: This will be initialized as DataSource PDA
    #[account(
        init,
        payer = authority,
        space = DataSource::SPACE,
        seeds = [
            DataSource::SEED_PREFIX,
            &authority.key().as_ref(),
            &data.name_seed(),
            data.data_source_type.to_seed(),
        ],
        bump
    )]
    pub data_source: Account<'info, DataSource>,

    pub system_program: Program<'info, System>,
}

impl DataSourceInfo {
    pub fn name_seed(&self) -> [u8; 32] {
        to_hash(&self.name)
    }
}