use crate::{
    error::MolphaError,
    state::{MAX_EPOCH_LEN_SECONDS, MAX_TICKETS_PER_EPOCH, MIN_EPOCH_LEN_SECONDS},
};
use anchor_lang::prelude::*;

pub const MIN_SIGNERS_FLOOR: u8 = 3;

/// Hard ceiling on `max_settlement_delay_seconds` (7 days).
///
/// The delay is both the attestation freshness window at `submit_attestation` ingress and the
/// grace period a superseded registry version stays active for. Capping it bounds how long a
/// removed node's key can still verify under an older snapshot.
pub const MAX_SETTLEMENT_DELAY_CAP_SECONDS: u64 = 604_800;

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    /// Published x402 pricing input: `x402_round_base + (k + b) · reward_per_signature`.
    /// Not read on chain; payouts come from the epoch pool.
    pub reward_per_signature: u64,
    /// Max age in seconds of `timestamp` relative to cluster time at attestation
    /// ingress, and the grace period of a superseded registry version.
    pub max_settlement_delay_seconds: u64,
    pub challenger_bounty_bps: u16,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_slots: u64,
    /// Published x402 pricing input; see `reward_per_signature`.
    pub x402_round_base: u64,
    /// Sum of every `Node.claimable_rewards`.
    pub reward_liability: u64,
    /// Current feed-admission floor; copied into each new epoch for ticket verification.
    pub min_signers: u8,
    /// Length of epochs created from now on.
    pub epoch_len_seconds: u64,
    /// Late-ticket window after an epoch ends, for epochs created from now on.
    pub ticket_grace_seconds: u64,
    /// Ticket count per epoch the threshold controller aims for.
    pub target_tickets_per_epoch: u32,
    /// Payout floor relative to the network inclusion rate, in bps, for new epochs.
    pub min_availability_bps: u16,
    /// Selections below which the availability floor does not apply, for new epochs.
    pub min_availability_samples: u16,
    /// Share of all swept income kept by the protocol; the rest joins the epoch pool.
    pub protocol_fee_bps: u16,
    /// Sum of `pool - paid` over every epoch not yet closed.
    pub pool_liability: u64,
    /// Protocol fees and slashing proceeds held in the treasury.
    pub protocol_reserved: u64,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    /// Upper bound for every basis-point knob.
    pub const MAX_BPS: u16 = 10_000;

    /// Shared invariants for initialization and every config update. Zero reward,
    /// base price, fees and challenger bounty are supported policies.
    pub fn validate(&self) -> Result<()> {
        require!(
            self.withdrawal_cooldown_slots > 0,
            MolphaError::InvalidWithdrawalCooldown
        );
        let now = Clock::get()?;
        require!(
            self.max_settlement_delay_seconds > 0
                && self.max_settlement_delay_seconds <= MAX_SETTLEMENT_DELAY_CAP_SECONDS
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
            self.minimum_node_deposit > 0,
            MolphaError::InvalidMinimumNodeDeposit
        );
        require!(
            self.min_signers >= MIN_SIGNERS_FLOOR,
            MolphaError::InvalidMinSigners
        );
        require!(
            self.challenger_bounty_bps <= Self::MAX_BPS,
            MolphaError::InvalidChallengerBounty
        );
        // Every possible selected signer count must have a representable x402 price.
        require!(
            self.reward_per_signature
                .checked_mul(crate::state::MAX_REGISTRY_NODES as u64)
                .and_then(|reward| self.x402_round_base.checked_add(reward))
                .is_some(),
            MolphaError::InvalidRoundPrice
        );
        require!(
            (MIN_EPOCH_LEN_SECONDS..=MAX_EPOCH_LEN_SECONDS).contains(&self.epoch_len_seconds),
            MolphaError::InvalidEpochLength
        );
        require!(
            self.ticket_grace_seconds > 0 && self.ticket_grace_seconds < self.epoch_len_seconds,
            MolphaError::InvalidTicketGrace
        );
        require!(
            (1..=MAX_TICKETS_PER_EPOCH / 8).contains(&self.target_tickets_per_epoch),
            MolphaError::InvalidTicketTarget
        );
        require!(
            self.min_availability_bps <= Self::MAX_BPS && self.min_availability_samples > 0,
            MolphaError::InvalidAvailabilityFloor
        );
        require!(
            self.protocol_fee_bps <= Self::MAX_BPS,
            MolphaError::InvalidProtocolFee
        );
        Ok(())
    }

    pub fn get_grace_active_until(&self) -> i64 {
        let now = Clock::get().unwrap().unix_timestamp;
        now.checked_add(self.max_settlement_delay_seconds as i64)
            .unwrap()
    }

    /// Treasury funds already owed: claimable rewards, uncredited pools and the protocol share.
    /// Anything above this in the treasury is unswept income.
    pub fn committed(&self) -> Result<u64> {
        self.reward_liability
            .checked_add(self.pool_liability)
            .and_then(|sum| sum.checked_add(self.protocol_reserved))
            .ok_or_else(|| error!(MolphaError::RewardOverflow))
    }

    /// Records swept income: the protocol keeps `protocol_fee_bps` of `gross`, the rest joins the
    /// epoch pool. Returns `(protocol_share, pool_share)`.
    pub fn record_income(&mut self, gross: u64) -> Result<(u64, u64)> {
        let protocol_share = crate::state::bps_of(gross, self.protocol_fee_bps);
        let pool_share = gross - protocol_share;
        self.protocol_reserved = self
            .protocol_reserved
            .checked_add(protocol_share)
            .ok_or(MolphaError::RewardOverflow)?;
        self.pool_liability = self
            .pool_liability
            .checked_add(pool_share)
            .ok_or(MolphaError::RewardOverflow)?;
        Ok((protocol_share, pool_share))
    }

    pub fn seeds(&self) -> [&[u8]; 2] {
        [Self::SEED_PREFIX, core::slice::from_ref(&self.bump)]
    }
}
