use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const LIVELINESS_SCORE_MAX: u16 = 10_000;
pub const LIVELINESS_PENALTY_PER_MISS: u16 = 500;
pub const LIVELINESS_RECOVERY_PER_SIGN: u16 = 100;
pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";

/// Permanent node identity and reward account; never closed or re-keyed.
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
    pub liveliness_score: u16,
    pub registered_at: i64,
    pub withdrawable_slot: u64,
    pub frozen_until: i64,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

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
        self.liveliness_score = LIVELINESS_SCORE_MAX;
        self.registered_at = registered_at;
        self.withdrawable_slot = 0;
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
