use anchor_lang::prelude::*;

/// Owner-keyed management/lifecycle account for a node operator.
///
/// Keyed by `owner` (`[SEED_PREFIX, owner]`), it holds registration, deposit, status, and
/// connectivity data — everything that is *not* needed by the per-round workflows. The workflow
/// data (registry slot, coalition key, reward balances) lives on the slot-keyed [`super::Node`].
/// `claimable_rewards` is the landing bucket for rewards migrated out of the virtual slot when a
/// removal transition is finalized; it lets a deactivated operator still claim.
#[account]
#[derive(InitSpace)]
pub struct NodeOperator {
    pub owner: Pubkey,
    pub current_index: u32,
    pub status: NodeStatus,
    pub ip: [u8; 4],
    pub port: u16,
    pub locked_amount: u64,
    /// Rewards migrated from the virtual slot at transition close, withdrawable via
    /// `claim_node_rewards` after deactivation.
    pub claimable_rewards: u64,
    pub registered_at: i64,
    pub deactivated_at: i64,
    pub withdrawable_at: i64,
    pub bump: u8,
}

impl NodeOperator {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node_operator";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    /// Initialize a freshly created operator account at registration time.
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
