use anchor_lang::prelude::*;

/// Owner-scoped delegate authorization with persistent job/request quotas.
#[account]
#[derive(InitSpace)]
pub struct Delegate {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub max_jobs_created: u32,
    pub jobs_created: u32,
    pub max_data_requests: u64,
    pub data_requests_used: u64,
    pub bump: u8,
}

impl Delegate {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_delegate";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        owner: Pubkey,
        delegate: Pubkey,
        max_jobs_created: u32,
        max_data_requests: u64,
        bump: u8,
    ) {
        self.owner = owner;
        self.delegate = delegate;
        self.max_jobs_created = max_jobs_created;
        self.jobs_created = 0;
        self.max_data_requests = max_data_requests;
        self.data_requests_used = 0;
        self.bump = bump;
    }

    pub fn try_consume_job(&mut self) -> Result<()> {
        require!(
            self.jobs_created < self.max_jobs_created,
            crate::error::MolphaError::DelegateJobLimitReached
        );
        self.jobs_created = self
            .jobs_created
            .checked_add(1)
            .ok_or(crate::error::MolphaError::DelegateJobLimitReached)?;
        Ok(())
    }

    pub fn try_consume_data_request(&mut self) -> Result<()> {
        require!(
            self.data_requests_used < self.max_data_requests,
            crate::error::MolphaError::DelegateDataRequestLimitReached
        );
        self.data_requests_used = self
            .data_requests_used
            .checked_add(1)
            .ok_or(crate::error::MolphaError::DelegateDataRequestLimitReached)?;
        Ok(())
    }
}
