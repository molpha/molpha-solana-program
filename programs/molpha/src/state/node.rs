use anchor_lang::prelude::*;

pub use molpha_verifier::state::VIRTUAL_INDEX;

#[account]
#[derive(InitSpace)]
pub struct Node {
    pub owner: Pubkey,
    pub current_index: u32,
    pub status: NodeStatus,
    pub ip: [u8; 4],
    pub port: u16,
    pub locked_amount: u64,
    /// Accrued rewards not yet past the dispute window.
    pub pending_rewards: u64,
    /// Finalized rewards, withdrawable via `claim_node_rewards`.
    pub claimable_rewards: u64,
    pub registered_at: i64,
    pub deactivated_at: i64,
    pub withdrawable_at: i64,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    /// Initialize a freshly created node account at registration time.
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        owner: Pubkey,
        current_index: u32,
        ip: [u8; 4],
        port: u16,
        locked_amount: u64,
        registered_at: i64,
        bump: u8,
    ) {
        self.owner = owner;
        self.current_index = current_index;
        self.status = NodeStatus::Active;
        self.ip = ip;
        self.port = port;
        self.locked_amount = locked_amount;
        self.pending_rewards = 0;
        self.claimable_rewards = 0;
        self.registered_at = registered_at;
        self.deactivated_at = 0;
        self.withdrawable_at = 0;
        self.bump = bump;
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
