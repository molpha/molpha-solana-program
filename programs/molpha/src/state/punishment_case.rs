use anchor_lang::prelude::*;

use super::OffenceKind;
use super::Evidence;

/// Permanent, on-chain record of an executed node punishment.
///
/// The PDA is seeded by `evidence_hash`, so `init` doubles as the anti-replay guard: the same
/// evidence can only ever be submitted once, independent of any per-offence terminal-status guard.
#[account]
#[derive(InitSpace)]
pub struct PunishmentCase {
    pub offence: OffenceKind,
    pub reporter: Pubkey,
    /// Registry version the guilty bitmap indexes into.
    pub registry_version: u32,
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub slashed_total: u64,
    pub bounty_paid: u64,
    pub evidence_hash: [u8; 32],
    pub executed_slot: u64,
    pub bump: u8,
}

impl PunishmentCase {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_punishment";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        evidence: &dyn Evidence,
        reporter: Pubkey,
        guilty_bitmap: [u8; 32],
        guilty_count: u16,
        slashed_total: u64,
        bounty_paid: u64,
        executed_slot: u64,
        bump: u8,
    ) {
        self.offence = evidence.offence_kind();
        self.reporter = reporter;
        self.registry_version = evidence.registry_version();
        self.guilty_bitmap = guilty_bitmap;
        self.guilty_count = guilty_count;
        self.slashed_total = slashed_total;
        self.bounty_paid = bounty_paid;
        self.evidence_hash = evidence.evidence_hash();
        self.executed_slot = executed_slot;
        self.bump = bump;
    }
}

// `OffenceKind` is a fieldless enum, so appending a variant (e.g. `NonceReuse`) never changes its
// `InitSpace` — this pins that invariant so a future offence with data fails loudly here instead
// of silently resizing every `PunishmentCase` account.
const _: () = assert!(PunishmentCase::SPACE == 136);
