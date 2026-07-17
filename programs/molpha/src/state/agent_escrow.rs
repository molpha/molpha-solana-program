use anchor_lang::prelude::*;

/// Lifecycle status shared by `Delegate` and `AgentEscrow`.
#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
pub enum AgentStatus {
    #[default]
    Active,
    Paused,
}

#[account]
#[derive(InitSpace)]
pub struct AgentEscrow {
    pub authority: Pubkey,
    pub status: AgentStatus,
    pub bump: u8,
}

impl AgentEscrow {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_agent";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, authority: Pubkey, bump: u8) {
        self.authority = authority;
        self.status = AgentStatus::Active;
        self.bump = bump;
    }
}
