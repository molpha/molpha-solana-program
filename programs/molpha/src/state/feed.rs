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

pub struct FeedInitParams {
    pub job_id: [u8; 32],
    pub value: [u8; 32],
    pub canonical_timestamp: i64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub bump: u8,
}

pub struct FeedUpdateParams {
    pub value: [u8; 32],
    pub canonical_timestamp: i64,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_feed";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, params: &FeedInitParams) {
        self.job_id = params.job_id;
        self.value = params.value;
        self.canonical_timestamp = params.canonical_timestamp;
        self.registry_version = params.registry_version;
        self.signers_bitmap = params.signers_bitmap;
        self.bump = params.bump;
    }

    pub fn update(&mut self, params: &FeedUpdateParams) {
        self.value = params.value;
        self.canonical_timestamp = params.canonical_timestamp;
        self.signatures_required = params.signatures_required;
        self.registry_version = params.registry_version;
        self.signers_bitmap = params.signers_bitmap;
        self.bump = params.bump;
    }
}
