use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Gateway {
    pub ip: [u8; 4],
    pub port: u16,
    pub registered_at: i64,
    pub bump: u8,
}

impl Gateway {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_gateway";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
