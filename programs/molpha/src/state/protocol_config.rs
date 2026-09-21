use crate::{error::MolphaError, state::LIVELINESS_SCORE_MAX};
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
    pub max_settlement_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_slots: u64,
    pub liveliness_freeze_threshold: u16,
    pub liveliness_penalty_per_miss: u16,
    pub liveliness_recovery_per_sign: u16,
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

    /// Shared invariants for initialization and every config update. Zero reward,
    /// base price, and challenger bounty are supported policies.
    pub fn validate(&self) -> Result<()> {
        require!(
            self.withdrawal_cooldown_slots > 0,
            MolphaError::InvalidWithdrawalCooldown
        );
        let now = Clock::get()?;
        require!(
            self.max_settlement_delay_seconds > 0
                && i64::try_from(self.max_settlement_delay_seconds)
                    .ok()
                    .and_then(|delay| now.unix_timestamp.checked_add(delay))
                    .is_some(),
            MolphaError::InvalidMaxReceiptDelay
        );
        require!(
            now.slot
                .checked_add(self.withdrawal_cooldown_slots)
                .is_some(),
            MolphaError::InvalidWithdrawalCooldown
        );
        require!(
            self.dispute_window_slots > 0
                && now.slot.checked_add(self.dispute_window_slots).is_some(),
            MolphaError::InvalidDisputeWindow
        );
        require!(
            self.minimum_node_deposit > 0,
            MolphaError::InvalidMinimumNodeDeposit
        );
        require!(self.gateway_bond_min > 0, MolphaError::InvalidGatewayBond);
        require!(
            self.min_signers >= MIN_SIGNERS_FLOOR,
            MolphaError::InvalidMinSigners
        );
        require!(
            self.challenger_bounty_bps <= Self::MAX_BPS,
            MolphaError::InvalidChallengerBounty
        );
        require!(
            (1..=LIVELINESS_SCORE_MAX).contains(&self.liveliness_freeze_threshold),
            MolphaError::InvalidLivelinessFreezeThreshold
        );
        require!(
            (1..=LIVELINESS_SCORE_MAX).contains(&self.liveliness_penalty_per_miss),
            MolphaError::InvalidLivelinessPenalty
        );
        require!(
            (1..=LIVELINESS_SCORE_MAX).contains(&self.liveliness_recovery_per_sign),
            MolphaError::InvalidLivelinessRecovery
        );
        // Every possible selected signer count must have a representable x402 price.
        require!(
            self.reward_per_signature
                .checked_mul(crate::state::MAX_REGISTRY_NODES as u64)
                .and_then(|reward| self.x402_round_base.checked_add(reward))
                .is_some(),
            MolphaError::InvalidRoundPrice
        );
        Ok(())
    }

    pub fn get_grace_active_until(&self) -> i64 {
        let now = Clock::get().unwrap().unix_timestamp;
        now.checked_add(self.max_settlement_delay_seconds as i64)
            .unwrap()
    }

    pub fn seeds(&self) -> [&[u8]; 2] {
        [Self::SEED_PREFIX, core::slice::from_ref(&self.bump)]
    }
}
