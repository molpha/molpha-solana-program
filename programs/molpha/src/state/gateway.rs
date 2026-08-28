use anchor_lang::prelude::*;

/// Bonded, Ed25519-signing gateway (v0.2 spec `gateway.authority`).
///
/// Holds endpoint metadata, bond, and reward accounting. Keyed by the gateway's Solana
/// authority key. `index` is the `gateway_index` referenced by `GatewayReceipt`.
#[account]
#[derive(InitSpace)]
pub struct Gateway {
    pub authority: Pubkey,
    pub index: u32,
    pub ip: [u8; 4],
    pub port: u16,
    pub slashed: bool,
    pub registered_at: i64,
    pub deactivated_at: i64,
    pub withdrawable_slot: u64,
    pub bump: u8,
}

impl Gateway {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_gateway";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
