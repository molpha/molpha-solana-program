use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub reward_per_signature: u64,
    pub max_receipt_delay_slots: u64,
    /// Fixed protocol fee per settled x402 round.
    pub protocol_fee: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub max_node_count: u32,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_secs: i64,
    pub x402_round_base: u64,
    /// Ops should keep pricing solvent vs `reward_per_signature` and `protocol_fee`.
    pub x402_round_per_sig: u64,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
