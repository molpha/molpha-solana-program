use anchor_lang::prelude::*;

use crate::utils::hash::to_hash;

#[account]
#[derive(Default, InitSpace)]
pub struct Node {
    pub authority: Pubkey,   // Who can manage this node
    pub node_pubkey: Pubkey, // The actual node's public key
    #[max_len(33)]
    pub node_pub_evm: Vec<u8>, // compressed secp256k1
    pub url_hash: [u8; 32],
    pub stake_amount: u64,
    pub security_deposit: u64, // Amount of security deposit locked
    pub status: NodeStatus,
    pub cooldown_until: i64,
    pub created_at: i64, // When the node was created
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        authority: Pubkey,
        node_pubkey: Pubkey,
        evm_pubkey: Vec<u8>,
        url: String,
    ) -> Result<()> {
        self.authority = authority;
        self.node_pubkey = node_pubkey;
        self.node_pub_evm = evm_pubkey;
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
