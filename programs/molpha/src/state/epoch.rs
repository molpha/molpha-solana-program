use anchor_lang::prelude::*;
use molpha_verifier::bitmap::{for_each_set_bit, Bitmap};

use crate::{
    error::MolphaError,
    state::{Registry, MAX_REGISTRY_NODES},
};

/// Hard cap on accepted tickets per epoch. Bounds marker rent float and keeps every per-node
/// counter within `u16`, which keeps [`EpochState`] under the 10 KiB CPI account-creation limit.
pub const MAX_TICKETS_PER_EPOCH: u32 = u16::MAX as u32;
/// Lowest ticket threshold the controller may set (sampling probability 2^-32).
pub const TAU_MIN: u64 = 1 << 32;
/// Largest factor the ticket threshold may move by in one adjustment, in either direction.
pub const TAU_STEP: u64 = 8;
pub const MIN_EPOCH_LEN_SECONDS: u64 = 3_600;
pub const MAX_EPOCH_LEN_SECONDS: u64 = 30 * 86_400;

const BPS: u128 = 10_000;

/// One epoch of the reward pool: its window, pinned ticket threshold, swept income and the
/// per-node participation counters fed by tickets.
///
/// Counters are keyed by an epoch-local roster of `Node` PDAs rather than by registry position,
/// because registry positions move between versions (swap-remove) while one epoch spans several.
/// The roster fills lazily as tickets name nodes for the first time.
///
/// PDA: `[SEED_PREFIX, epoch.to_le_bytes()]`. Layout has no implicit padding.
#[account(zero_copy)]
#[repr(C)]
pub struct EpochState {
    pub epoch: u64,
    /// Inclusive window start (unix seconds). Equals the previous epoch's `end`.
    pub start: i64,
    /// Exclusive window end (unix seconds).
    pub end: i64,
    /// Snapshot of `ticket_grace_seconds` at creation.
    pub ticket_grace: i64,
    /// A round is a ticket when the big-endian `u64` prefix of its ticket hash is below this.
    pub tau: u64,
    /// USDC swept into this epoch while it was open.
    pub pool: u64,
    /// Sum of payouts credited to nodes.
    pub paid: u64,
    /// `Σ weight(slot)`, frozen at finalization.
    pub weight_total: u64,
    pub tickets: u32,
    /// `Σ signed`, maintained per ticket.
    pub sum_signed: u32,
    /// `Σ selected`, maintained per ticket.
    pub sum_selected: u32,
    /// Snapshot: availability floor relative to the network inclusion rate, in bps.
    pub min_availability_bps: u16,
    /// Snapshot: samples below which the availability floor does not apply.
    pub min_samples: u16,
    pub roster_len: u16,
    /// Roster slots with non-zero weight, frozen at finalization.
    pub weighted_count: u16,
    /// Weighted slots already processed by `credit_epoch_rewards`.
    pub credited_count: u16,
    pub finalized: u8,
    pub bump: u8,
    /// Receives this account's rent when the epoch is closed.
    pub rent_payer: [u8; 32],
    /// Bitmap over roster slots already credited.
    pub credited: [u8; 32],
    pub signed: [u16; MAX_REGISTRY_NODES],
    pub selected: [u16; MAX_REGISTRY_NODES],
    pub roster: [[u8; 32]; MAX_REGISTRY_NODES],
    /// Snapshot of the admission quorum floor for rounds counted in this epoch.
    pub min_signers: u8,
    pub _reserved: [u8; 7],
}

const _: () = assert!(core::mem::size_of::<EpochState>() == 9_376);
const _: () = assert!(EpochState::SPACE <= 10_240);

/// Where a roster hint resolved to.
enum RosterSlot {
    Counted(usize),
    /// Roster full; the node is not counted for this epoch.
    Overflow,
}

impl EpochState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_epoch";
    pub const SPACE: usize = 8 + core::mem::size_of::<Self>();

    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        epoch: u64,
        start: i64,
        end: i64,
        ticket_grace: i64,
        tau: u64,
        min_availability_bps: u16,
        min_samples: u16,
        min_signers: u8,
        rent_payer: Pubkey,
        bump: u8,
    ) {
        self.epoch = epoch;
        self.start = start;
        self.end = end;
        self.ticket_grace = ticket_grace;
        self.tau = tau;
        self.min_availability_bps = min_availability_bps;
        self.min_samples = min_samples;
        self.min_signers = min_signers;
        self.rent_payer = rent_payer.to_bytes();
        self.bump = bump;
    }

    pub fn is_finalized(&self) -> bool {
        self.finalized != 0
    }

    /// Whether a round with this `canonical_timestamp` belongs to the epoch.
    pub fn contains(&self, canonical_timestamp: u64) -> bool {
        i64::try_from(canonical_timestamp)
            .map(|ts| self.start <= ts && ts < self.end)
            .unwrap_or(false)
    }

    /// Sweeps and close-epoch residuals credit the epoch open at cluster time.
    pub fn is_open(&self, now: i64) -> bool {
        !self.is_finalized() && self.start <= now && now < self.end
    }

    pub fn ticket_deadline(&self) -> i64 {
        self.end.saturating_add(self.ticket_grace)
    }

    pub fn accepts_tickets(&self, now: i64) -> bool {
        !self.is_finalized() && now < self.ticket_deadline()
    }

    pub fn finalizable(&self, now: i64) -> bool {
        !self.is_finalized() && now >= self.ticket_deadline()
    }

    /// Closing waits one further epoch length after the ticket deadline so inactivity evidence,
    /// which reads this account, can still be filed.
    pub fn closable(&self, now: i64) -> bool {
        self.is_finalized()
            && self.credited_count == self.weighted_count
            && now
                >= self
                    .ticket_deadline()
                    .saturating_add(self.end.saturating_sub(self.start))
    }

    pub fn is_ticket(&self, ticket_hash: &[u8; 32]) -> bool {
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&ticket_hash[..8]);
        u64::from_be_bytes(prefix) < self.tau
    }

    pub fn roster_key(&self, slot: usize) -> Result<Pubkey> {
        require!(
            slot < usize::from(self.roster_len),
            MolphaError::InvalidRosterSlot
        );
        Ok(Pubkey::new_from_array(self.roster[slot]))
    }

    /// Whether the node at `slot` falls below the availability floor: at least `min_samples`
    /// selections and an inclusion rate `signed / selected` below `min_availability_bps` of the
    /// network-wide rate `sum_signed / sum_selected`.
    ///
    /// The floor is relative because only `k` of the `k + b` selected nodes sign a round, so even
    /// a perfectly available node sits near `k / (k + b)`.
    pub fn below_availability_floor(&self, slot: usize) -> bool {
        let signed = u128::from(self.signed[slot]);
        let selected = u128::from(self.selected[slot]);
        selected >= u128::from(self.min_samples)
            && signed * u128::from(self.sum_selected) * BPS
                < u128::from(self.min_availability_bps) * u128::from(self.sum_signed) * selected
    }

    /// Payout weight: sampled signature count, zeroed below the availability floor.
    pub fn weight(&self, slot: usize) -> u64 {
        if self.below_availability_floor(slot) {
            0
        } else {
            u64::from(self.signed[slot])
        }
    }

    pub fn is_credited(&self, slot: usize) -> bool {
        self.credited[slot / 8] & (1 << (slot % 8)) != 0
    }

    pub fn mark_credited(&mut self, slot: usize) {
        self.credited[slot / 8] |= 1 << (slot % 8);
    }

    /// Adds one ticket's selection to the counters. `hints[i]` names the roster slot of the
    /// i-th selected registry position (ascending bit order): an existing slot, or `roster_len`
    /// to append a node the epoch has not seen yet. Returns how many selected nodes were not
    /// counted because the roster is full.
    pub fn record_ticket(
        &mut self,
        registry: &Registry,
        selection: Bitmap,
        signers: &Bitmap,
        hints: &[u16],
    ) -> Result<u32> {
        require!(
            hints.len() == selection.popcount() as usize,
            MolphaError::InvalidRosterSlot
        );

        let mut cursor = 0usize;
        let mut overflowed = 0u32;
        for_each_set_bit(selection, |bit| {
            let hint = usize::from(hints[cursor]);
            cursor += 1;

            let node = registry.node(bit)?.to_bytes();
            let slot = match self.resolve_slot(hint, &node)? {
                RosterSlot::Counted(slot) => slot,
                RosterSlot::Overflow => {
                    overflowed += 1;
                    return Ok(());
                }
            };

            self.selected[slot] = self.selected[slot]
                .checked_add(1)
                .ok_or(MolphaError::TicketCapReached)?;
            self.sum_selected = self
                .sum_selected
                .checked_add(1)
                .ok_or(MolphaError::TicketCapReached)?;
            if signers.bit_set(bit) {
                self.signed[slot] = self.signed[slot]
                    .checked_add(1)
                    .ok_or(MolphaError::TicketCapReached)?;
                self.sum_signed = self
                    .sum_signed
                    .checked_add(1)
                    .ok_or(MolphaError::TicketCapReached)?;
            }
            Ok::<(), Error>(())
        })?;

        Ok(overflowed)
    }

    fn resolve_slot(&mut self, hint: usize, node: &[u8; 32]) -> Result<RosterSlot> {
        let roster_len = usize::from(self.roster_len);
        if hint < roster_len {
            require!(&self.roster[hint] == node, MolphaError::InvalidRosterSlot);
            return Ok(RosterSlot::Counted(hint));
        }

        // Appending (or overflowing) is only legal for a node the roster does not hold yet;
        // otherwise a submitter could leave a known node uncounted.
        require!(hint == roster_len, MolphaError::InvalidRosterSlot);
        require!(
            !self.roster[..roster_len].iter().any(|key| key == node),
            MolphaError::InvalidRosterSlot
        );
        if roster_len == MAX_REGISTRY_NODES {
            return Ok(RosterSlot::Overflow);
        }

        self.roster[roster_len] = *node;
        self.roster_len += 1;
        Ok(RosterSlot::Counted(roster_len))
    }

    /// Freezes the payout weights. Returns `(weight_total, weighted_count)`.
    pub fn finalize(&mut self) -> (u64, u16) {
        let mut weight_total = 0u64;
        let mut weighted_count = 0u16;
        for slot in 0..usize::from(self.roster_len) {
            let weight = self.weight(slot);
            if weight > 0 {
                // At most 256 slots of at most u16::MAX each: cannot overflow.
                weight_total += weight;
                weighted_count += 1;
            }
        }
        self.weight_total = weight_total;
        self.weighted_count = weighted_count;
        self.finalized = 1;
        (weight_total, weighted_count)
    }

    /// `floor(pool · weight / weight_total)`.
    pub fn payout(&self, weight: u64) -> u64 {
        if self.weight_total == 0 {
            return 0;
        }
        (u128::from(self.pool) * u128::from(weight) / u128::from(self.weight_total)) as u64
    }

    /// Pool left after payouts: forfeited shares and rounding dust.
    pub fn residual(&self) -> u64 {
        self.pool.saturating_sub(self.paid)
    }

    /// Threshold for the epoch two ahead: scale by `target / observed`, move at most
    /// [`TAU_STEP`]× either way, and stay within `[TAU_MIN, u64::MAX]`.
    pub fn next_tau(&self, target_tickets: u32) -> u64 {
        let tau = u128::from(self.tau);
        let observed = u128::from(self.tickets.max(1));
        let step = u128::from(TAU_STEP);
        (tau * u128::from(target_tickets) / observed)
            .clamp(tau / step, tau * step)
            .clamp(u128::from(TAU_MIN), u128::from(u64::MAX)) as u64
    }
}
