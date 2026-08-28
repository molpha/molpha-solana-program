use anchor_lang::prelude::*;

use crate::error::{map_attestation_accounts_err, map_attestation_err};

use super::Registry;

/// Permanent node identity and reward account; never closed or re-keyed.
#[account]
#[derive(InitSpace)]
pub struct Node {
    pub owner: Pubkey,
    pub secp256k1_pubkey_x: [u8; 32],
    pub secp256k1_pubkey_y: [u8; 32],
    pub status: NodeStatus,
    pub ip: [u8; 4],
    pub port: u16,
    pub locked_amount: u64,
    pub claimable_rewards: u64,
    pub registered_at: i64,
    pub deactivated_at: i64,
    pub withdrawable_at: i64,
    pub frozen_until: i64,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        owner: Pubkey,
        secp256k1_pubkey_x: [u8; 32],
        secp256k1_pubkey_y: [u8; 32],
        ip: [u8; 4],
        port: u16,
        locked_amount: u64,
        registered_at: i64,
        bump: u8,
    ) {
        self.owner = owner;
        self.secp256k1_pubkey_x = secp256k1_pubkey_x;
        self.secp256k1_pubkey_y = secp256k1_pubkey_y;
        self.status = NodeStatus::Active;
        self.ip = ip;
        self.port = port;
        self.locked_amount = locked_amount;
        self.claimable_rewards = 0;
        self.registered_at = registered_at;
        self.deactivated_at = 0;
        self.withdrawable_at = 0;
        self.frozen_until = 0;
        self.bump = bump;
    }

    pub fn update_endpoint(&mut self, ip: [u8; 4], port: u16) {
        self.ip = ip;
        self.port = port;
    }
}

#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
pub enum NodeStatus {
    #[default]
    Active,
    Deactivated,
    /// Evicted from the registry by a punishment with `MembershipAction::Freeze`; restorable via
    /// `reinstate_node` once `frozen_until` has passed (or immediately by the protocol authority).
    Frozen,
    /// Evicted from the registry by a punishment with `MembershipAction::Tombstone`. Terminal.
    Tombstoned,
}

/// Resolve signer accounts against a registry snapshot in ascending bit order.
pub fn resolve_registry_signers(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
    registry: &Registry,
    signers_bitmap: &[u8; 32],
) -> Result<Vec<molpha_verifier::SignerXy>> {
    let entries = molpha_verifier::solana::resolve_nodes(accounts, program_id)
        .map_err(map_attestation_accounts_err)?;
    molpha_verifier::resolve_registry_signers(
        &registry.nodes,
        registry.node_count,
        signers_bitmap,
        &entries,
    )
    .map_err(map_attestation_err)
}

/// Like [`resolve_registry_signers`], but also returns each signer's bit position.
pub fn resolve_registry_signers_indexed(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
    registry: &Registry,
    signers_bitmap: &[u8; 32],
) -> Result<Vec<(usize, molpha_verifier::SignerXy)>> {
    let entries = molpha_verifier::solana::resolve_nodes(accounts, program_id)
        .map_err(map_attestation_accounts_err)?;
    molpha_verifier::resolve_registry_signers_indexed(
        &registry.nodes,
        registry.node_count,
        signers_bitmap,
        &entries,
    )
    .map_err(map_attestation_err)
}
