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
    /// Rounds settled against this job (v0.2 §4.2).
    pub used_rounds: u64,
    /// Round quota; `u64::MAX` means unlimited (gated by subscription validity).
    pub max_rounds: u64,
    pub bump: u8,
}

impl Job {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_job";
    pub const MAX_DELEGATES: usize = 5;
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        job_id: [u8; 32],
        owner: Pubkey,
        api_config_hash: [u8; 32],
        decimals: u8,
        signatures_required: u8,
        created_at: i64,
        bump: u8,
    ) {
        self.job_id = job_id;
        self.owner = owner;
        self.delegates = [Pubkey::default(); 5];
        self.delegate_count = 0;
        self.api_config_hash = api_config_hash;
        self.decimals = decimals;
        self.created_at = created_at;
        self.signatures_required = signatures_required;
        self.used_rounds = 0;
        self.max_rounds = u64::MAX;
        self.bump = bump;
    }

    pub fn has_delegate(&self, delegate: &Pubkey) -> bool {
        self.delegates[..self.delegate_count as usize]
            .iter()
            .any(|d| d == delegate)
    }
}
