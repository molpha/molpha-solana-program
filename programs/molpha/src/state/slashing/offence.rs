use anchor_lang::prelude::*;

use molpha_verifier::bitmap::Bitmap;

use crate::error::MolphaError;
use crate::state::{Node, NodeStatus, ProtocolConfig, Registry};

use super::{
    CommitmentReuseEvidence, EquivocationEvidence, InactivityEvidence, NonceReuseEvidence,
    UnsolicitedSigningEvidence,
};

/// Accounts an offence needs in order to prove itself.
pub struct EvidenceAccounts<'info, 'a> {
    pub remaining: &'info [AccountInfo<'info>],
    pub evidence_registry: &'a AccountLoader<'info, Registry>,
    pub protocol_config: &'a ProtocolConfig,
    pub current_registry_version: u32,
}

/// Node-fault offence types. Each carries a fixed [`Penalty`] — see [`OffenceKind::penalty`].
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub enum Offence {
    /// Two valid Molpha attestations for the same execution duty with different values.
    Equivocation(EquivocationEvidence),
    /// A valid Molpha attestation whose signer bitmap contains a node outside the duty's
    /// deterministic selection set.
    UnsolicitedSigning(UnsolicitedSigningEvidence),
    /// A node contributed a partial signature under one aggregate nonce commitment for two
    /// different signed messages.
    CommitmentReuse(CommitmentReuseEvidence),
    /// A node produced two verifying partial signatures against one nonce pair it had committed
    /// in a signed batch. Key-compromise-equivalent: a third such partial extracts the share.
    NonceReuse(NonceReuseEvidence),
    /// A node's availability in a finalized epoch fell below that epoch's payout floor.
    Inactivity(InactivityEvidence),
}

/// Fieldless offence identity stored on `PunishmentCase` and in events.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum OffenceKind {
    Equivocation,
    UnsolicitedSigning,
    CommitmentReuse,
    NonceReuse,
    Inactivity,
}

/// Registry-membership consequence of a punishment.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum MembershipAction {
    None,
    Freeze,
    Tombstone,
}

/// Freeze window for [`OffenceKind::Inactivity`]: one day, after which anyone may `reinstate_node`.
pub const INACTIVITY_FREEZE_SECS: i64 = 86_400;

/// What a proven offence costs. Shared across offence types.
#[derive(Clone, Copy, Debug)]
pub struct Penalty {
    /// Portion of the node's slashable balances destroyed, in bps. `10_000` = 100%.
    pub slash_bps: u16,
    pub membership: MembershipAction,
    /// For [`MembershipAction::Freeze`]: seconds until permissionless reinstatement is allowed.
    pub freeze_secs: Option<i64>,
}

/// Slashed amounts from a single [`Penalty::apply`] call.
#[derive(Clone, Copy, Debug, Default)]
pub struct PenaltyOutcome {
    pub slashed_locked: u64,
    pub slashed_rewards: u64,
}

impl PenaltyOutcome {
    pub fn total(&self) -> u64 {
        self.slashed_locked.saturating_add(self.slashed_rewards)
    }
}

pub fn bps_of(amount: u64, bps: u16) -> u64 {
    (amount as u128 * bps as u128 / 10_000) as u64
}

pub trait Evidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)>;
    fn hash(&self) -> [u8; 32];
    fn registry_version(&self) -> u32;
}

impl Offence {
    pub fn evidence(&self) -> &dyn Evidence {
        match self {
            Self::Equivocation(e) => e,
            Self::UnsolicitedSigning(e) => e,
            Self::CommitmentReuse(e) => e,
            Self::NonceReuse(e) => e,
            Self::Inactivity(e) => e,
        }
    }

    pub fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        self.evidence().validate(ctx)
    }

    pub fn hash(&self) -> [u8; 32] {
        self.evidence().hash()
    }

    pub fn evidence_hash(&self) -> [u8; 32] {
        self.hash()
    }

    pub fn registry_version(&self) -> u32 {
        self.evidence().registry_version()
    }

    pub const fn kind(&self) -> OffenceKind {
        match self {
            Self::Equivocation(_) => OffenceKind::Equivocation,
            Self::UnsolicitedSigning(_) => OffenceKind::UnsolicitedSigning,
            Self::CommitmentReuse(_) => OffenceKind::CommitmentReuse,
            Self::NonceReuse(_) => OffenceKind::NonceReuse,
            Self::Inactivity(_) => OffenceKind::Inactivity,
        }
    }

    pub const fn penalty(&self) -> Penalty {
        self.kind().penalty()
    }
}

impl OffenceKind {
    pub const fn penalty(self) -> Penalty {
        match self {
            Self::Equivocation => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: None,
            },
            Self::UnsolicitedSigning => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: None,
            },
            Self::CommitmentReuse => Penalty {
                slash_bps: 5_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: None,
            },
            Self::NonceReuse => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: None,
            },
            Self::Inactivity => Penalty {
                slash_bps: 0,
                membership: MembershipAction::Freeze,
                freeze_secs: Some(INACTIVITY_FREEZE_SECS),
            },
        }
    }
}

impl Penalty {
    pub fn apply(&self, node: &mut Node, now: i64) -> Result<PenaltyOutcome> {
        require!(
            node.status != NodeStatus::Tombstoned,
            MolphaError::NodeTombstoned
        );
        let slashed_locked = bps_of(node.locked_amount, self.slash_bps);
        let slashed_rewards = bps_of(node.claimable_rewards, self.slash_bps);
        node.locked_amount = node.locked_amount.saturating_sub(slashed_locked);
        node.claimable_rewards = node.claimable_rewards.saturating_sub(slashed_rewards);

        self.apply_membership_action(node, now);

        Ok(PenaltyOutcome {
            slashed_locked,
            slashed_rewards,
        })
    }

    fn apply_membership_action(&self, node: &mut Node, now: i64) {
        match self.membership {
            MembershipAction::None => {}
            MembershipAction::Freeze => {
                node.status = NodeStatus::Frozen;
                node.frozen_until = now.saturating_add(self.freeze_secs.unwrap_or(0));
            }
            MembershipAction::Tombstone => {
                node.status = NodeStatus::Tombstoned;
            }
        }
    }
}
