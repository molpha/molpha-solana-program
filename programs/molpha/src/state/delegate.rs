use anchor_lang::prelude::*;

/// Owner-scoped delegate authorization. `max_data_requests` is enforced by the gateway at
/// admission, not on chain.
#[account]
#[derive(InitSpace)]
pub struct Delegate {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub max_data_requests: u64,
    pub bump: u8,
}

impl Delegate {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_delegate";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, owner: Pubkey, delegate: Pubkey, max_data_requests: u64, bump: u8) {
        self.owner = owner;
        self.delegate = delegate;
        self.max_data_requests = max_data_requests;
        self.bump = bump;
    }
}
