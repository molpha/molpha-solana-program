use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Feed {
    pub job_id: [u8; 32],
    pub registry_version: u32,
    pub canonical_timestamp: i64,
    pub value: [u8; 32],
    pub signers_bitmap: [u8; 32],
    pub signatures_required: u8,
    pub last_updated_slot: u64,
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_feed";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, job_id: [u8; 32], bump: u8) {
        self.job_id = job_id;
        self.registry_version = 0;
        self.canonical_timestamp = 0;
        self.value = [0u8; 32];
        self.signers_bitmap = [0u8; 32];
        self.signatures_required = 0;
        self.last_updated_slot = 0;
        self.bump = bump;
    }
}
