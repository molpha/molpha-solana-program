use anchor_lang::prelude::*;
use molpha_verifier::SignerXy;

use crate::utils::registry_account::RegistryAllocation;
use crate::{
    error::MolphaError,
    state::{Registry, RegistryBuilder, RegistryMut, RegistryRef, MAX_REGISTRY_NODES},
};

/// Mutable pointer to the current immutable registry snapshot.
#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub current_version: u32,
    pub next_version: u32,
    pub bump: u8,
    /// The id of the next provider to register. Starts at 1 and only grows: ids are never reused.
    pub next_provider_id: u32,
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
    /// Cluster time of the transition: the next version's `active_from`.
    pub now: i64,
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, bump: u8) {
        self.next_version = 1;
        self.bump = bump;
        self.next_provider_id = 1;
    }

    /// Hand out the next provider id.
    pub fn take_provider_id(&mut self) -> Result<u32> {
        let id = self.next_provider_id;
        self.next_provider_id = id
            .checked_add(1)
            .ok_or(error!(crate::error::MolphaError::ProviderIdExhausted))?;
        Ok(id)
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

        // Room for every current record, and for the ones being added. A removal leaves the table
        // larger than it needs, and an insert that takes a hole adds no record; it is shrunk once
        // the final record count is known. The caps themselves are enforced where a slot is added.
        let slots = transition.current.load()?.records() + added;
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
                transition.now,
                transition.bump,
            )?;
            let result = apply(&mut builder)?;
            (result, builder.header.records())
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

    /// Publish a version with a provider added, carrying its signing key: in the lowest free
    /// provider slot. A new slot past the last one adds its record (and slot 0, the reserved hole,
    /// the first time); a freed slot adds none. Returns the slot.
    pub fn insert_provider(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        provider: Pubkey,
        key: SignerXy,
    ) -> Result<u32> {
        let (expected_slot, providers) = {
            let current = RegistryRef::load(transition.current.as_ref())?;
            (
                current
                    .first_free_provider_slot()
                    .ok_or(error!(MolphaError::ProviderSlotsFull))?,
                usize::from(current.provider_count),
            )
        };
        let added = (expected_slot + 1).saturating_sub(providers);
        self.publish(transition, added, |next| {
            let slot = next.insert_provider(provider, key)?;
            debug_assert_eq!(slot, expected_slot);
            Ok(u32::try_from(slot).unwrap())
        })
    }

    /// Publish a version without `provider`. Its record becomes a hole; every other member keeps
    /// its slot. Returns the provider slot it held.
    pub fn remove_provider(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        provider: Pubkey,
    ) -> Result<u32> {
        self.publish(transition, 0, |next| {
            Ok(u32::try_from(next.remove_provider(provider)?).unwrap())
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

    /// Publish a version without `nodes`. Returns the node slot count of the new version.
    pub fn remove_nodes(
        &mut self,
        transition: &RegistryTransition<'_, '_>,
        nodes: &[Pubkey],
    ) -> Result<u8> {
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
