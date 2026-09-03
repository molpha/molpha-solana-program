use anchor_lang::prelude::*;

pub const MIN_SIGNERS_FLOOR: u8 = 3;

/// Seed of the PDA that owns the shared node-deposit vault.
///
/// Deliberately not `protocol_config`: that PDA signs every treasury payout, so giving the
/// deposits a different authority means a reward or bounty instruction cannot reach node
/// principal even by mistake.
pub const NODE_DEPOSITS_SEED: &[u8] = b"molpha_node_deposits";

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
    pub x402_round_base: u64,
    /// Sum of every `Node.claimable_rewards`. The treasury must always hold at least this much:
    /// node principal lives in per-node deposit vaults, so anything above this line is protocol
    /// revenue and anything below it means credited rewards are unbacked.
    pub reward_liability: u64,
    /// `Σ Node.locked_amount`, and the balance the node-deposit vault must always hold.
    ///
    /// Node principal lives in one vault, separate from the treasury and under its own authority,
    /// so no reward or bounty payout can reach it. This counter makes that backing auditable from
    /// state alone. It reconciles the vault against the accounting; the per-node bound is enforced
    /// separately, at each debit, against that node's own `locked_amount`.
    pub total_node_deposits: u64,
    pub min_signers: u8,
    pub bump: u8,
    /// Bump for [`NODE_DEPOSITS_SEED`], the deposit vault's token authority.
    pub node_deposits_bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    /// Upper bound for every basis-point knob.
    pub const MAX_BPS: u16 = 10_000;
}
