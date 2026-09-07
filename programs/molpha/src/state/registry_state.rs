use anchor_lang::prelude::*;

/// Mutable pointer to the current immutable registry snapshot.
#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub current_version: u32,
    pub next_version: u32,
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, bump: u8) {
        self.current_version = 0;
        self.next_version = 1;
        self.bump = bump;
    }

    pub fn advance_version(&mut self) {
        self.current_version = self.next_version;
        self.next_version = self.next_version.checked_add(1).unwrap();
    }

    pub fn seeds(&self) -> [&[u8]; 2] {
        [Self::SEED_PREFIX, core::slice::from_ref(&self.bump)]
    }
}
