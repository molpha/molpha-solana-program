use core::cell::{Ref, RefMut};
use core::ops::Deref;

use anchor_lang::prelude::*;
use anchor_lang::Discriminator;
use molpha_verifier::{NodeEntry, RegistryView, SignerXy};

use crate::error::MolphaError;

/// Width of a signer bitmap, and so the most members a registry can hold.
pub const MAX_REGISTRY_NODES: usize = 256;

/// One member of a registry snapshot: the node account and the secp256k1 key it signed with
/// while this version was current.
///
/// The key is copied from the `Node` when the member enters the snapshot and changes only when a
/// new version replaces it, so a version keeps verifying what the key it names signed. Layout is
/// the verifier's `NodeEntry`.
#[zero_copy]
#[repr(C)]
pub struct RegistryMember {
    pub node: [u8; 32],
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl RegistryMember {
    pub const LEN: usize = core::mem::size_of::<Self>();

    pub fn new(node: Pubkey, (x, y): SignerXy) -> Self {
        Self {
            node: node.to_bytes(),
            x,
            y,
        }
    }

    pub fn xy(&self) -> SignerXy {
        (self.x, self.y)
    }

    /// A hole: the slot of a member that was removed. Live members always carry a node account.
    pub fn is_vacant(&self) -> bool {
        // Four word reads: a byte-wise compare of 32 bytes costs far more on SBF.
        let a = &self.node;
        let word = |i: usize| {
            u64::from_ne_bytes([
                a[i],
                a[i + 1],
                a[i + 2],
                a[i + 3],
                a[i + 4],
                a[i + 5],
                a[i + 6],
                a[i + 7],
            ])
        };
        (word(0) | word(8) | word(16) | word(24)) == 0
    }

    pub const VACANT: Self = Self {
        node: [0; 32],
        x: [0; 32],
        y: [0; 32],
    };
}

/// Header of an immutable, version-addressed registry snapshot: the selection policy and how
/// many slots the snapshot has.
///
/// The members themselves follow the header in the same account, one [`RegistryMember`] each. A
/// member's position is its slot, the bit it signs with, and it keeps that slot for as long as it
/// is in a snapshot: removing a node leaves a hole (a vacant record) rather than moving the last
/// member into it, and a later registration takes the lowest hole. `node_count` is the number of
/// slots, holes included; trailing holes are trimmed, so the last slot is always a live member.
/// The account is sized to hold exactly `node_count` records ([`Registry::space`]), so a snapshot
/// costs what its slots need and the table's length is the account's. That is also how the verifier
/// reads it. Use [`RegistryRef`] to read a snapshot with its members; this struct alone gives the
/// header.
#[account(zero_copy)]
#[repr(C)]
pub struct Registry {
    pub grace_active_until: i64,
    /// Inclusive creation time. A version cannot verify a round dated before it existed.
    pub active_from: i64,
    pub version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub bump: u8,
}

impl Registry {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    /// Bytes before the member table: the Anchor discriminator and this header.
    pub const HEADER_SPACE: usize = 8 + core::mem::size_of::<Self>();

    /// Account size of a snapshot with room for `members` members.
    pub const fn space(members: usize) -> usize {
        Self::HEADER_SPACE + members * RegistryMember::LEN
    }

    /// Members an account of `data_len` bytes has room for, or `None` when that length is not a
    /// header followed by a whole number of member records.
    pub const fn capacity_of(data_len: usize) -> Option<usize> {
        if data_len < Self::HEADER_SPACE {
            return None;
        }
        let table = data_len - Self::HEADER_SPACE;
        if !table.is_multiple_of(RegistryMember::LEN) {
            return None;
        }
        Some(table / RegistryMember::LEN)
    }

    pub fn init_genesis(&mut self, redundancy_buffer: u8, active_from: i64, bump: u8) {
        self.set_redundancy_buffer(redundancy_buffer).unwrap();
        self.active_from = active_from;
        self.bump = bump;
    }

    /// Update the redundancy buffer on a snapshot that was already copied from the previous registry.
    pub fn set_redundancy_buffer(&mut self, redundancy_buffer: u8) -> Result<()> {
        require!(
            redundancy_buffer > 0 && redundancy_buffer <= 8,
            MolphaError::InvalidRedundancyBuffer
        );
        require!(
            redundancy_buffer != self.redundancy_buffer,
            MolphaError::RegistryPolicyUnchanged
        );
        self.redundancy_buffer = redundancy_buffer;
        Ok(())
    }

    pub fn is_active(&self, at: i64) -> bool {
        self.active_from <= at && (self.grace_active_until == 0 || self.grace_active_until > at)
    }
}

/// Slot of `node` among `members`. A hole never matches, not even the default pubkey.
fn index_of(members: &[RegistryMember], node: Pubkey) -> Option<usize> {
    let node = node.to_bytes();
    if node == [0u8; 32] {
        return None;
    }
    members.iter().position(|member| member.node == node)
}

/// Split registry account data into its header and member table. `None` unless the data is a
/// header followed by a whole number of member records.
fn split(data: &[u8]) -> Option<(&[u8], &[u8])> {
    Registry::capacity_of(data.len())?;
    let (head, table) = data.split_at(Registry::HEADER_SPACE);
    Some((&head[Registry::DISCRIMINATOR.len()..], table))
}

/// A registry snapshot read from its account: the header and the members it names.
///
/// Derefs to the header.
pub struct RegistryRef<'a> {
    header: Ref<'a, Registry>,
    table: Ref<'a, [RegistryMember]>,
}

impl<'a> RegistryRef<'a> {
    /// Borrow an initialised registry account, checking owner, discriminator and shape.
    pub fn load(account: &'a AccountInfo<'_>) -> Result<Self> {
        require_keys_eq!(
            *account.owner,
            crate::ID,
            ErrorCode::AccountOwnedByWrongProgram
        );
        let data = account.try_borrow_data()?;
        require!(
            data.len() >= Registry::DISCRIMINATOR.len()
                && data[..Registry::DISCRIMINATOR.len()] == *Registry::DISCRIMINATOR,
            ErrorCode::AccountDiscriminatorMismatch
        );
        let valid = split(&data).is_some_and(|(header, table)| {
            usize::from(bytemuck::from_bytes::<Registry>(header).node_count)
                <= table.len() / RegistryMember::LEN
        });
        require!(valid, ErrorCode::AccountDidNotDeserialize);

        let (header, table) = Ref::map_split(data, |data| {
            let (header, table) = split(data).expect("shape checked above");
            (
                bytemuck::from_bytes::<Registry>(header),
                bytemuck::cast_slice::<u8, RegistryMember>(table),
            )
        });
        Ok(Self { header, table })
    }

    /// The snapshot's slots, in registry order. A removed member's slot is a vacant record.
    pub fn members(&self) -> &[RegistryMember] {
        &self.table[..usize::from(self.header.node_count)]
    }

    /// The live member in slot `index`; a hole or a slot past the table is an invalid index.
    pub fn member(&self, index: usize) -> Result<RegistryMember> {
        self.members()
            .get(index)
            .copied()
            .filter(|member| !member.is_vacant())
            .ok_or(error!(MolphaError::InvalidNodeIndex))
    }

    /// How many slots hold a live member.
    pub fn live_count(&self) -> usize {
        self.members().iter().filter(|m| !m.is_vacant()).count()
    }

    pub fn node(&self, index: usize) -> Result<Pubkey> {
        Ok(Pubkey::new_from_array(self.member(index)?.node))
    }

    pub fn get_index(&self, node: Pubkey) -> Result<usize> {
        index_of(self.members(), node).ok_or(error!(MolphaError::InvalidNodeAccount))
    }

    pub fn is_node_in_registry(&self, node: Pubkey) -> bool {
        index_of(self.members(), node).is_some()
    }

    /// Framework-agnostic view for [`molpha_verifier`], with the member table viewed in place.
    /// Selection needs the table too: it draws over the live members and skips the holes.
    pub fn view(&self) -> RegistryView<'_> {
        let table = NodeEntry::slice_from_bytes(bytemuck::cast_slice(&self.table[..]))
            .expect("a member record is one verifier NodeEntry");
        RegistryView {
            version: self.header.version,
            node_count: self.header.node_count,
            redundancy_buffer: self.header.redundancy_buffer,
            nodes: table,
        }
    }
}

impl Deref for RegistryRef<'_> {
    type Target = Registry;

    fn deref(&self) -> &Registry {
        &self.header
    }
}

/// A registry account borrowed for writing: its header and its whole member table.
pub struct RegistryMut<'a> {
    header: RefMut<'a, Registry>,
    table: RefMut<'a, [RegistryMember]>,
}

impl<'a> RegistryMut<'a> {
    /// Borrow an initialised registry account.
    pub fn load(account: &'a AccountInfo<'_>) -> Result<Self> {
        Self::borrow(account, false)
    }

    /// Borrow a registry account that has been allocated but never written, and mark it as a
    /// registry. Its length must already be a header plus a whole number of member records.
    pub fn load_init(account: &'a AccountInfo<'_>) -> Result<Self> {
        Self::borrow(account, true)
    }

    fn borrow(account: &'a AccountInfo<'_>, init: bool) -> Result<Self> {
        require_keys_eq!(
            *account.owner,
            crate::ID,
            ErrorCode::AccountOwnedByWrongProgram
        );
        let mut data = account.try_borrow_mut_data()?;
        require!(split(&data).is_some(), ErrorCode::AccountDidNotDeserialize);
        let discriminator = Registry::DISCRIMINATOR;
        if init {
            require!(
                data[..discriminator.len()].iter().all(|byte| *byte == 0),
                ErrorCode::AccountDiscriminatorAlreadySet
            );
            data[..discriminator.len()].copy_from_slice(discriminator);
        } else {
            require!(
                data[..discriminator.len()] == *discriminator,
                ErrorCode::AccountDiscriminatorMismatch
            );
        }

        let (header, table) = RefMut::map_split(data, |data| {
            let (head, table) = data.split_at_mut(Registry::HEADER_SPACE);
            (
                bytemuck::from_bytes_mut::<Registry>(&mut head[discriminator.len()..]),
                bytemuck::cast_slice_mut::<u8, RegistryMember>(table),
            )
        });
        require!(
            usize::from(header.node_count) <= table.len(),
            ErrorCode::AccountDidNotDeserialize
        );
        Ok(Self { header, table })
    }

    /// The populated members, in registry order.
    pub fn members(&self) -> &[RegistryMember] {
        &self.table[..usize::from(self.header.node_count)]
    }

    /// The header for writing, together with the populated members.
    pub fn parts(&mut self) -> (&mut Registry, &[RegistryMember]) {
        let count = usize::from(self.header.node_count);
        (&mut self.header, &self.table[..count])
    }

    /// The snapshot as something to build on: its header and the whole table it has room for.
    pub fn builder(&mut self) -> RegistryBuilder<'_> {
        RegistryBuilder {
            header: &mut self.header,
            members: &mut self.table,
        }
    }
}

/// A snapshot under construction: its header, and a member table whose length is the room the
/// account has. Every operation keeps `members[..node_count]` as the slots (live members and
/// holes, the last one live) and the rest zeroed.
pub struct RegistryBuilder<'a> {
    pub header: &'a mut Registry,
    pub members: &'a mut [RegistryMember],
}

impl RegistryBuilder<'_> {
    fn populated(&self) -> &[RegistryMember] {
        &self.members[..usize::from(self.header.node_count)]
    }

    /// Start from the previous snapshot: the same members and policy, one version on. The
    /// previous snapshot is stamped with the time its grace ends, the only change it ever sees.
    pub fn copy_from_previous(
        &mut self,
        previous: &mut Registry,
        previous_members: &[RegistryMember],
        grace_active_until: i64,
        active_from: i64,
        bump: u8,
    ) -> Result<()> {
        require!(
            previous_members.len() == usize::from(previous.node_count)
                && previous_members.len() <= self.members.len(),
            MolphaError::NodeCapExceeded
        );
        self.header.version = previous.version + 1;
        self.header.node_count = previous.node_count;
        self.header.redundancy_buffer = previous.redundancy_buffer;
        self.header.bump = bump;
        self.header.active_from = active_from;
        self.header.grace_active_until = 0;
        self.members[..previous_members.len()].copy_from_slice(previous_members);
        previous.grace_active_until = grace_active_until;
        Ok(())
    }

    /// Add a node and its key in the lowest vacant slot, or past the last slot when there is no
    /// hole. Returns the slot.
    pub fn insert_node(&mut self, node: Pubkey, key: SignerXy) -> Result<usize> {
        require!(
            !self.is_node_in_registry(node),
            MolphaError::NodeAlreadyInRegistry
        );
        let count = usize::from(self.header.node_count);
        let slot = self
            .populated()
            .iter()
            .position(RegistryMember::is_vacant)
            .unwrap_or(count);
        require!(
            slot < MAX_REGISTRY_NODES && slot < self.members.len(),
            MolphaError::NodeCapExceeded
        );

        self.members[slot] = RegistryMember::new(node, key);
        if slot == count {
            // `count` is below 256, so the new count fits.
            self.header.node_count = (count + 1) as u16;
        }
        Ok(slot)
    }

    /// Replace a member's key. The member keeps its slot. Returns that slot.
    pub fn replace_key(&mut self, node: Pubkey, key: SignerXy) -> Result<usize> {
        let index = self.get_index(node)?;
        self.members[index] = RegistryMember::new(node, key);
        Ok(index)
    }

    /// Remove a node, leaving its slot vacant.
    pub fn remove_node(&mut self, node: Pubkey) -> Result<()> {
        let index = self.get_index(node)?;
        self.vacate(index);
        Ok(())
    }

    /// Remove every listed node that is a member, skipping the ones that are not. Every other
    /// member keeps its slot.
    pub fn remove_nodes(&mut self, remove_nodes: &[Pubkey]) -> Result<()> {
        for remove_node in remove_nodes {
            if let Some(index) = index_of(self.populated(), *remove_node) {
                self.vacate(index);
            }
        }
        Ok(())
    }

    pub fn get_index(&self, node: Pubkey) -> Result<usize> {
        index_of(self.populated(), node).ok_or(error!(MolphaError::InvalidNodeAccount))
    }

    pub fn is_node_in_registry(&self, node: Pubkey) -> bool {
        index_of(self.populated(), node).is_some()
    }

    /// Empty a slot, then drop the trailing holes so the table ends on a live member.
    fn vacate(&mut self, index: usize) {
        self.members[index] = RegistryMember::VACANT;
        let mut count = usize::from(self.header.node_count);
        while count > 0 && self.members[count - 1].is_vacant() {
            count -= 1;
        }
        self.header.node_count = count as u16;
    }
}

const _: () = assert!(RegistryMember::LEN == NodeEntry::LEN);
const _: () = assert!(Registry::HEADER_SPACE == 32);
// One instruction can create an account of up to 10 KiB, which is a snapshot of 106 members.
// A larger one is grown first by `extend_registry`.
const _: () = assert!(Registry::space(106) <= 10_240 && Registry::space(107) > 10_240);
const _: () = assert!(Registry::space(MAX_REGISTRY_NODES) == 24_608);
