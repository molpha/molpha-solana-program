use anchor_lang::prelude::*;

use crate::utils::hash::to_hash;

#[account]
#[derive(Default, InitSpace)]
pub struct Node {
    pub authority: Pubkey,   // Who can manage this node
    pub node_pubkey: Pubkey, // The actual node's public key
    /// secp256k1 public key (affine) stored as raw x/y coordinates.
    /// This avoids per-verify decompression of compressed keys.
    pub node_pub_evm_x: [u8; 32],
    pub node_pub_evm_y: [u8; 32],
    pub url_hash: [u8; 32],
    pub stake_amount: u64,
    pub security_deposit: u64, // Amount of security deposit locked
    pub status: NodeStatus,
    pub cooldown_until: i64,
    pub created_at: i64, // When the node was created
    /// 1-based stable index matching `signers_bitmap` bit positions (EVM-style).
    pub node_index: u32,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        authority: Pubkey,
        node_pubkey: Pubkey,
        evm_pubkey_xy: ([u8; 32], [u8; 32]),
        url: String,
    ) -> Result<()> {
        self.authority = authority;
        self.node_pubkey = node_pubkey;
        self.node_pub_evm_x = evm_pubkey_xy.0;
        self.node_pub_evm_y = evm_pubkey_xy.1;
        self.url_hash = to_hash(&url);
        self.status = NodeStatus::Registered;
        self.created_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default)]
pub enum NodeStatus {
    #[default]
    Registered,
    Active,
    Exited,
    Jailed,
    Slashed,
}
