use anchor_lang::prelude::*;

/// Protocol-authorized gateway identified by its Solana authority.
///
/// Holds endpoint metadata and activation status. Keyed by the gateway's authority key, which
/// signs the round dispatches tickets must carry. Gateways hold no funds: x402 payers pay the
/// protocol treasury directly.
#[account]
#[derive(InitSpace)]
pub struct Gateway {
    pub authority: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub status: GatewayStatus,
    pub registered_at: i64,
    pub bump: u8,
}

#[derive(InitSpace, AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GatewayStatus {
    Active,
    Deactivated,
}

impl Gateway {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_gateway";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn seeds(&self) -> [&[u8]; 3] {
        [
            Self::SEED_PREFIX,
            self.authority.as_ref(),
            core::slice::from_ref(&self.bump),
        ]
    }
}
