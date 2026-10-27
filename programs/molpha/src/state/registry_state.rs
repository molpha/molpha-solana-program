use anchor_lang::prelude::*;
use molpha_verifier::SignerXy;

use crate::state::{Registry, RegistryBuilder, RegistryMut, MAX_REGISTRY_NODES};
use crate::utils::registry_account::RegistryAllocation;

/// Mutable pointer to the current immutable registry snapshot.
#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub current_version: u32,
    pub next_version: u32,
    pub bump: u8,
}

/// The accounts a registry transition reads and writes.
pub struct RegistryTransition<'a, 'info> {
    pub current: &'a AccountLoader<'info, Registry>,
    /// The next version's PDA. Not created yet, or allocated by `extend_registry`.
    pub next: &'a AccountInfo<'info>,
    /// Funds the next version's rent.
    pub payer: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
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

    /// Publish the next registry version: copy the current snapshot into the next account, run
    /// `apply` on the copy, and size the account to the members it ends up with.
    ///
    /// `added` is how many members `apply` may add; one that takes a hole adds no slot.
    fn publish<F, R>(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        added: usize,
        apply: F,
    ) -> Result<R>
    where
        F: FnOnce(&mut RegistryBuilder<'_>) -> Result<R>,
    {
        let allocation = RegistryAllocation {
            account: transition.next,
            payer: transition.payer,
            system_program: transition.system_program,
            version: self.next_version,
            bump: transition.bump,
        };

        // Room for every current slot, and for the ones being added. A removal leaves the table
        // larger than it needs, and an insert that takes a hole adds no slot; it is shrunk once
        // the final slot count is known. The cap itself is enforced where a slot is added.
        let slots = usize::from(transition.current.load()?.node_count) + added;
        allocation.resize_to(Registry::space(slots.min(MAX_REGISTRY_NODES)))?;

        let (result, slot_count) = {
            let mut current = RegistryMut::load(transition.current.as_ref())?;
            let mut next = RegistryMut::load_init(transition.next)?;
            let (previous, previous_members) = current.parts();
            let mut builder = next.builder();
            builder.copy_from_previous(
                previous,
                previous_members,
                transition.grace_active_until,
                Clock::get()?.unix_timestamp,
                transition.bump,
            )?;
            let result = apply(&mut builder)?;
            (result, usize::from(builder.header.node_count))
        };
        allocation.settle(Registry::space(slot_count))?;

        self.current_version = self.next_version;
        self.next_version = self.next_version.checked_add(1).unwrap();
        Ok(result)
    }

    /// Publish a version with `node` added, carrying its signing key: in the lowest vacant slot,
    /// or after the last slot when there is none. Returns its slot.
    pub fn insert_node(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        node: Pubkey,
        key: SignerXy,
    ) -> Result<u32> {
        self.publish(transition, 1, |next| {
            let slot = next.insert_node(node, key)?;
            Ok(u32::try_from(slot).unwrap())
        })
    }

    /// Publish a version in which `node` signs with `key`, at the index it already holds.
    /// Returns that index. Every other member, and the policy, carry over unchanged.
    pub fn replace_node_key(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        node: Pubkey,
        key: SignerXy,
    ) -> Result<u32> {
        self.publish(transition, 0, |next| {
            let index = next.replace_key(node, key)?;
            Ok(u32::try_from(index).unwrap())
        })
    }

    /// Publish a version without `node`. Its slot becomes a hole; every other member keeps its slot.
    pub fn remove_node(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        node: Pubkey,
    ) -> Result<()> {
        self.publish(transition, 0, |next| next.remove_node(node))
    }

    /// Publish a version without `nodes`. Returns the slot count of the new version.
    pub fn remove_nodes(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        nodes: &[Pubkey],
    ) -> Result<u16> {
        self.publish(transition, 0, |next| {
            next.remove_nodes(nodes)?;
            Ok(next.header.node_count)
        })
    }

    pub fn set_redundancy_buffer(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        redundancy_buffer: u8,
    ) -> Result<()> {
        self.publish(transition, 0, |next| {
            next.header.set_redundancy_buffer(redundancy_buffer)
        })
    }

    pub fn seeds(&self) -> [&[u8]; 2] {
        [Self::SEED_PREFIX, core::slice::from_ref(&self.bump)]
    }
}
