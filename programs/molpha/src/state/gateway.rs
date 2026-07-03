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
    /// USDC bond held in the protocol treasury, slashable on a successful dispute.
    pub bond: u64,
    /// Accrued but not-yet-finalized fees.
    pub pending_rewards: u64,
    /// Finalized fees, withdrawable via `claim_gateway_rewards`.
    pub claimable_rewards: u64,
    /// Pre-funded balance that finances agent rounds paid over external rails (x402):
    /// the agent pays the gateway off-chain, and `settle_agent_round` debits the round
    /// price from here. Topped up via `deposit_gateway_settlement`; custody is the
    /// protocol treasury (internal accounting, like `bond`). Withdrawal is Phase 1.5.
    pub settlement_balance: u64,
    pub slashed: bool,
    pub registered_at: i64,
    pub bump: u8,
}

impl Gateway {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_gateway";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
