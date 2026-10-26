use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";

/// Permanent node identity and reward account; never closed.
///
/// `secp256k1_pubkey_{x,y}` is the node's current signing key. Rotating it publishes a registry
/// version that names the new key; versions already published keep the key they were created
/// with, so nothing signed under them stops verifying.
#[account]
#[derive(InitSpace)]
pub struct Node {
    pub authority: Pubkey,
    pub secp256k1_pubkey_x: [u8; 32],
    pub secp256k1_pubkey_y: [u8; 32],
    pub status: NodeStatus,
    pub ip: [u8; 4],
    pub port: u16,
    pub locked_amount: u64,
    pub claimable_rewards: u64,
    pub registered_at: i64,
    pub withdrawable_slot: u64,
    pub frozen_until: i64,
    pub bump: u8,
    /// When the signing key was last rotated; `0` if it never was.
    pub key_rotated_at: i64,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn add_rewards(&mut self, amount: u64) -> Result<()> {
        self.claimable_rewards = self
            .claimable_rewards
            .checked_add(amount)
            .ok_or(crate::error::MolphaError::RewardOverflow)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        authority: Pubkey,
        secp256k1_pubkey_x: [u8; 32],
        secp256k1_pubkey_y: [u8; 32],
        ip: [u8; 4],
        port: u16,
        locked_amount: u64,
        registered_at: i64,
        bump: u8,
    ) {
        self.authority = authority;
        self.secp256k1_pubkey_x = secp256k1_pubkey_x;
        self.secp256k1_pubkey_y = secp256k1_pubkey_y;
        self.status = NodeStatus::Active;
        self.ip = ip;
        self.port = port;
        self.locked_amount = locked_amount;
        self.claimable_rewards = 0;
        self.registered_at = registered_at;
        self.withdrawable_slot = 0;
        self.frozen_until = 0;
        self.bump = bump;
        self.key_rotated_at = 0;
    }

    /// Switch to a new signing key. The caller publishes the registry version that names it.
    pub fn set_key(&mut self, (x, y): ([u8; 32], [u8; 32]), rotated_at: i64) {
        self.secp256k1_pubkey_x = x;
        self.secp256k1_pubkey_y = y;
        self.key_rotated_at = rotated_at;
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

pub fn derive_node_id(
    owner: Pubkey,
    secp256k1_pubkey_compressed: &[u8; 33],
    registration_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        NODE_ID_PREFIX,
        owner.as_ref(),
        secp256k1_pubkey_compressed.as_ref(),
        registration_salt.as_ref(),
    ])
    .to_bytes()
}
