use anchor_lang::prelude::*;

pub const MIN_SIGNERS_FLOOR: u8 = 3;

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub reward_per_signature: u64,
    /// Max age in seconds of `canonical_timestamp` relative to cluster time at
    /// attestation/settlement ingress. Bounds the offline selection-grind space.
    pub max_receipt_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_slots: u64,
    pub liveliness_freeze_threshold: u16,
    pub x402_round_base: u64,
    /// Sum of every `Node.claimable_rewards`. The treasury must always hold at least this much
    pub reward_liability: u64,
    pub min_signers: u8,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    /// Upper bound for every basis-point knob.
    pub const MAX_BPS: u16 = 10_000;

    pub fn seeds(&self) -> [&[u8]; 2] {
        [
            Self::SEED_PREFIX,
            core::slice::from_ref(&self.bump),
        ]
    }
}
