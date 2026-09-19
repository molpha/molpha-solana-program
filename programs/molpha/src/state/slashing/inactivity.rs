use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::bitmap::Bitmap;

use crate::error::MolphaError;
use crate::state::{Evidence, EvidenceAccounts, Node, NodeStatus, MAX_REGISTRY_NODES};

pub const DOMAIN_INACTIVITY: &[u8] = b"MOLPHA_INACTIVITY_V1";

/// Identifies a node in the current registry; inactivity is proven from its on-chain state.
/// A later inactivity episode after reinstatement uses a new registry version and case hash.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InactivityEvidence {
    pub registry_version: u32,
    pub node_index: u32,
}

impl Evidence for InactivityEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        require!(
            self.registry_version == ctx.current_registry_version,
            MolphaError::RegistryVersionMismatch
        );
        require!(ctx.remaining.len() == 1, MolphaError::InvalidNodeAccount);

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
        require!(
            node.liveliness_score < ctx.protocol_config.liveliness_freeze_threshold,
            MolphaError::LivelinessAboveThreshold
        );

        let mut guilty_bitmap = Bitmap::new();
        guilty_bitmap.set_bit(node_index);
        Ok((guilty_bitmap, guilty_bitmap))
    }

    fn hash(&self) -> [u8; 32] {
        hashv(&[
            DOMAIN_INACTIVITY,
            self.registry_version.to_be_bytes().as_slice(),
            self.node_index.to_be_bytes().as_slice(),
        ])
        .to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.registry_version
    }
}
