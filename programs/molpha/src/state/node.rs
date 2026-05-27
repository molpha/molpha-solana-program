use anchor_lang::prelude::*;

pub const VIRTUAL_INDEX: u32 = u32::MAX;

#[account]
#[derive(InitSpace)]
pub struct Node {
    pub owner: Pubkey,
    pub current_index: u32,
    pub status: NodeStatus,
    pub ip: [u8; 4],
    pub port: u16,
    pub locked_amount: u64,
    pub pending_rewards: u64,
    pub registered_at: i64,
    pub deactivated_at: i64,
    pub withdrawable_at: i64,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
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
