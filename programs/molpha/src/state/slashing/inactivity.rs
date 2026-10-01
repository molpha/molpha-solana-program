use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::bitmap::Bitmap;

use crate::error::MolphaError;
use crate::state::{EpochState, Evidence, EvidenceAccounts, Node, NodeStatus, MAX_REGISTRY_NODES};

pub const DOMAIN_INACTIVITY: &[u8] = b"MOLPHA_INACTIVITY_V2";

/// A node whose availability in a finalized epoch fell below that epoch's payout floor.
///
/// Proven from the epoch's own counters; `registry_version` / `node_index` locate the node in the
/// current registry so it can be removed. The case hash covers only `(epoch, roster_slot)`, so a
/// node can be punished at most once per epoch, including after reinstatement.
///
/// Remaining accounts: `[node, epoch_state]`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InactivityEvidence {
    pub epoch: u64,
    pub roster_slot: u16,
    pub registry_version: u32,
    pub node_index: u32,
}

impl Evidence for InactivityEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        require!(
            self.registry_version == ctx.current_registry_version,
            MolphaError::RegistryVersionMismatch
        );
        require!(ctx.remaining.len() == 2, MolphaError::InvalidNodeAccount);

        let node_index =
            usize::try_from(self.node_index).map_err(|_| error!(MolphaError::InvalidNodeIndex))?;
        require!(
            node_index < MAX_REGISTRY_NODES,
            MolphaError::InvalidNodeIndex
        );

        let node: Account<Node> = Account::try_from(&ctx.remaining[0])?;
        require_keys_eq!(
            node.key(),
            ctx.evidence_registry.load()?.node(node_index)?,
            MolphaError::InvalidNodeAccount
        );
        require!(
            node.status == NodeStatus::Active,
            MolphaError::NodeNotActive
        );

        let epoch_loader = AccountLoader::<EpochState>::try_from(&ctx.remaining[1])?;
        let epoch = epoch_loader.load()?;
        require!(
            epoch.epoch == self.epoch && epoch.is_finalized(),
            MolphaError::EpochNotFinalized
        );
        let slot = usize::from(self.roster_slot);
        require_keys_eq!(
            node.key(),
            epoch.roster_key(slot)?,
            MolphaError::InvalidNodeAccount
        );
        // A freeze that overlapped this epoch already covers the outage it caused.
        require!(
            node.frozen_until <= epoch.start,
            MolphaError::InactivityAlreadyPunished
        );
        require!(
            epoch.below_availability_floor(slot),
            MolphaError::AvailabilityAboveFloor
        );

        let mut guilty_bitmap = Bitmap::new();
        guilty_bitmap.set_bit(node_index);
        Ok((guilty_bitmap, guilty_bitmap))
    }

    fn hash(&self) -> [u8; 32] {
        hashv(&[
            DOMAIN_INACTIVITY,
            self.epoch.to_be_bytes().as_slice(),
            self.roster_slot.to_be_bytes().as_slice(),
        ])
        .to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.registry_version
    }
}
