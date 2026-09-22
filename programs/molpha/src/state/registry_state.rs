use anchor_lang::prelude::*;

use crate::state::Registry;

/// Mutable pointer to the current immutable registry snapshot.
#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub current_version: u32,
    pub next_version: u32,
    pub bump: u8,
}

pub struct RegistryTransition<'a, 'b> {
    pub current: &'a AccountLoader<'b, Registry>,
    pub next: &'a AccountLoader<'b, Registry>,
    pub grace_active_until: i64,
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, bump: u8) {
        self.next_version = 1;
        self.bump = bump;
    }

    /// Initialize the next registry account, copy state from the current snapshot, then run `apply`.
    fn with_registry_snapshot<'a, 'b, F, R>(
        &mut self,
        transition: &RegistryTransition<'a, 'b>,
        apply: F,
    ) -> Result<R>
    where
        F: FnOnce(&Registry, &mut Registry) -> Result<R>,
    {
        let mut current = transition.current.load_mut()?;
        let mut next = transition.next.load_init()?;
        next.copy_from_previous(&mut current, transition.grace_active_until, transition.bump);
        self.current_version = self.next_version;
        self.next_version = self.next_version.checked_add(1).unwrap();
        apply(&current, &mut next)
    }

    pub fn append_node<'a, 'b>(
        &mut self,
        transition: &RegistryTransition<'a, 'b>,
        node: Pubkey,
    ) -> Result<u32> {
        self.with_registry_snapshot(transition, |current, next| {
            next.append_node(node)?;
            Ok(u32::from(current.node_count))
        })
    }

    pub fn swap_remove_node<'a, 'b>(
        &mut self,
        transition: &RegistryTransition<'a, 'b>,
        node: Pubkey,
    ) -> Result<()> {
        self.with_registry_snapshot(transition, |_, next| {
            next.swap_remove_node(node)?;
            Ok(())
        })
    }

    pub fn swap_remove_nodes<'a, 'b>(
        &mut self,
        transition: &RegistryTransition<'a, 'b>,
        nodes: &[Pubkey],
    ) -> Result<u16> {
        self.with_registry_snapshot(transition, |_, next| {
            next.swap_remove_nodes(nodes)?;
            Ok(next.node_count)
        })
    }

    pub fn set_redundancy_buffer<'a, 'b>(
        &mut self,
        transition: &RegistryTransition<'a, 'b>,
        redundancy_buffer: u8,
    ) -> Result<()> {
        self.with_registry_snapshot(transition, |_, next| {
            next.set_redundancy_buffer(redundancy_buffer)?;
            Ok(())
        })
    }

    pub fn seeds(&self) -> [&[u8]; 2] {
        [Self::SEED_PREFIX, core::slice::from_ref(&self.bump)]
    }
}
