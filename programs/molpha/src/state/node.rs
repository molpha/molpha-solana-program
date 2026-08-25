use anchor_lang::{prelude::*, Discriminator};
use borsh::BorshDeserialize;
use molpha_verifier::NodeEntry;

use crate::error::{map_attestation_accounts_err, map_attestation_err, MolphaError};

use super::Registry;

/// Permanent owner-keyed node identity, lifecycle, and reward account.
///
/// Historical registries retain this account address forever. Consequently the owner and
/// secp256k1 key fields are initialized once and never changed, and the account is never closed.
/// A tombstone binds this node *identity*, not the operator behind it — the same operator can
/// register a fresh wallet and a fresh secp256k1 key, which is inherent to permissionless
/// registration.
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
    /// Unix timestamp a `Frozen` node becomes permissionlessly reinstatable via
    /// `reinstate_node`. Unused for any other status.
    pub frozen_until: i64,
    /// Unix timestamp of the most recent punishment, or `0` if never punished.
    pub punished_at: i64,
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
        self.punished_at = 0;
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

/// Resolve selected signer accounts against an immutable registry snapshot.
///
/// Anchor I/O adapter: owner-checks and Borsh-deserializes `Node` accounts via [`resolve_nodes`],
/// then delegates slot binding to [`molpha_verifier::resolve_registry_signers`]. Accounts must be
/// provided in ascending signer-bit order. Node status is deliberately ignored: a node
/// deactivated in a later version remains valid evidence for historical versions.
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

/// Like [`resolve_registry_signers`] but also returns each signer's bit position.
///
/// Used by callers that need to re-partition a resolved union bitmap back into per-statement
/// subsets — e.g. equivocation punishment, which resolves the union of two signer sets once and
/// then splits the result back into each statement's ordered signers.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_node_roundtrips() {
        let account = Node {
            owner: Pubkey::new_from_array([0xaa; 32]),
            secp256k1_pubkey_x: [0x11; 32],
            secp256k1_pubkey_y: [0x22; 32],
            status: NodeStatus::Active,
            ip: [127, 0, 0, 1],
            port: 8080,
            locked_amount: 100,
            claimable_rewards: 9,
            registered_at: 1,
            deactivated_at: 0,
            withdrawable_at: 0,
            frozen_until: 0,
            punished_at: 0,
            bump: 7,
        };
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), Node::INIT_SPACE);

        let mut data = vec![0u8; Node::SPACE];
        data[..8].copy_from_slice(Node::DISCRIMINATOR);
        data[8..].copy_from_slice(&body);
        let decoded = Node::deserialize(&mut &data[8..]).unwrap();
        assert_eq!(decoded.owner, account.owner);
        assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
        assert_eq!(decoded.claimable_rewards, account.claimable_rewards);
    }
}
