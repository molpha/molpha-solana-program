use anchor_lang::prelude::*;
use molpha_verifier::bitmap::{bitmap_load, for_each_set_bit_u256};

use crate::error::MolphaError;

use super::Registry;

/// Permanent owner-keyed node identity, lifecycle, and reward account.
///
/// Historical registries retain this account address forever. Consequently the owner and
/// secp256k1 key fields are initialized once and never changed, and the account is never closed.
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
    Slashed,
    Exited,
}

/// Resolve selected signer accounts against an immutable registry snapshot.
///
/// Accounts must be provided in ascending signer-bit order. Node status is deliberately ignored:
/// a node deactivated in a later version remains valid evidence for historical versions.
pub fn resolve_registry_signers(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
    registry: &Registry,
    signers_bitmap: &[u8; 32],
) -> Result<Vec<molpha_verifier::SignerXy>> {
    let signers = bitmap_load(signers_bitmap);
    let signer_count = signers.count_ones() as usize;
    require!(
        accounts.len() == signer_count,
        MolphaError::MissingSignerAccount
    );

    let mut ordered = Vec::with_capacity(signer_count);
    let mut cursor = 0usize;
    for_each_set_bit_u256(signers, |bit_pos| {
        let registry_index = bit_pos as usize;
        require!(
            registry_index < usize::from(registry.node_count),
            MolphaError::InvalidSignersBitmap
        );

        let account = accounts
            .get(cursor)
            .ok_or(error!(MolphaError::MissingSignerAccount))?;
        cursor = cursor.saturating_add(1);
        require_keys_eq!(
            account.key(),
            Pubkey::new_from_array(registry.nodes[registry_index]),
            MolphaError::MissingSignerAccount
        );
        require_keys_eq!(
            *account.owner,
            *program_id,
            MolphaError::MissingSignerAccount
        );

        let data = account.try_borrow_data()?;
        let node = Node::try_deserialize(&mut &data[..])
            .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
        ordered.push((node.secp256k1_pubkey_x, node.secp256k1_pubkey_y));
        Ok(())
    })?;

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::Discriminator;

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
            bump: 7,
        };
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), Node::INIT_SPACE);

        let mut data = vec![0u8; Node::SPACE];
        data[..8].copy_from_slice(Node::DISCRIMINATOR);
        data[8..].copy_from_slice(&body);
        let decoded = Node::try_deserialize(&mut data.as_slice()).unwrap();
        assert_eq!(decoded.owner, account.owner);
        assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
        assert_eq!(decoded.claimable_rewards, account.claimable_rewards);
    }
}
