use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Feed {
    pub job_id: [u8; 32],
    pub value: [u8; 32],
    pub canonical_timestamp: i64,
    pub signatures_required: u8,
    pub signers_bitmap: [u8; 32],
    pub registry_version: u32,
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_feed";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        job_id: [u8; 32],
        value: [u8; 32],
        canonical_timestamp: i64,
        signatures_required: u8,
        registry_version: u32,
        signers_bitmap: [u8; 32],
        bump: u8,
    ) {
        self.job_id = job_id;
        self.value = value;
        self.canonical_timestamp = canonical_timestamp;
        self.signatures_required = signatures_required;
        self.registry_version = registry_version;
        self.signers_bitmap = signers_bitmap;
        self.bump = bump;
    }
}
