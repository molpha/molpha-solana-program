use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Job {
    pub job_id: [u8; 32],
    pub owner: Pubkey,
    pub delegates: [Pubkey; 5],
    pub delegate_count: u8,
    pub api_config_hash: [u8; 32],
    pub decimals: u8,
    pub signatures_required: u8,
    pub created_at: i64,
    pub bump: u8,
}

pub struct JobInitParams {
    pub job_id: [u8; 32],
    pub owner: Pubkey,
    pub api_config_hash: [u8; 32],
    pub decimals: u8,
    pub signatures_required: u8,
    pub created_at: i64,
    pub bump: u8,
}

impl Job {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_job";
    pub const MAX_DELEGATES: usize = 5;
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, params: &JobInitParams) {
        self.job_id = params.job_id;
        self.owner = params.owner;
        self.delegates = [Pubkey::default(); 5];
        self.delegate_count = 0;
        self.api_config_hash = params.api_config_hash;
        self.decimals = params.decimals;
        self.created_at = params.created_at;
        self.signatures_required = params.signatures_required;
        self.bump = params.bump;
    }

    pub fn has_delegate(&self, delegate: &Pubkey) -> bool {
        self.delegates[..self.delegate_count as usize]
            .iter()
            .any(|d| d == delegate)
    }
}
