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
}

/// Header of an immutable, version-addressed registry snapshot: the selection policy and how
/// many members the snapshot has.
///
/// The members themselves follow the header in the same account, one [`RegistryMember`] each, in
/// registry order. The account is sized to hold exactly `node_count` of them
/// ([`Registry::space`]), so a snapshot costs what its membership needs and the table's length is
/// the account's. That is also how the verifier reads it. Use [`RegistryRef`] to read a snapshot
/// with its members; this struct alone gives the header.
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

    /// View for selection derivation, which reads the version, the member count and the
    /// redundancy buffer but no member. Verification needs the members: see [`RegistryRef::view`].
    pub fn selection_view(&self) -> RegistryView<'static> {
        RegistryView {
            version: self.version,
            node_count: self.node_count,
            redundancy_buffer: self.redundancy_buffer,
            nodes: &[],
        }
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

/// Position of `node` among `members`.
fn index_of(members: &[RegistryMember], node: Pubkey) -> Option<usize> {
    let node = node.to_bytes();
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

    /// The snapshot's members, in registry order.
    pub fn members(&self) -> &[RegistryMember] {
        &self.table[..usize::from(self.header.node_count)]
    }

    pub fn member(&self, index: usize) -> Result<RegistryMember> {
        self.members()
            .get(index)
            .copied()
            .ok_or(error!(MolphaError::InvalidNodeIndex))
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
/// account has. Every operation keeps `members[..node_count]` populated and the rest zeroed.
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

    /// Append a node and its key.
    pub fn append_node(&mut self, node: Pubkey, key: SignerXy) -> Result<()> {
        let count = usize::from(self.header.node_count);
        require!(
            count < MAX_REGISTRY_NODES && count < self.members.len(),
            MolphaError::NodeCapExceeded
        );
        require!(
            !self.is_node_in_registry(node),
            MolphaError::NodeAlreadyInRegistry
        );

        self.members[count] = RegistryMember::new(node, key);
        // `count` is below 256, so the new count fits.
        self.header.node_count = (count + 1) as u16;
        Ok(())
    }

    /// Replace a member's key. The member keeps its index. Returns that index.
    pub fn replace_key(&mut self, node: Pubkey, key: SignerXy) -> Result<usize> {
        let index = self.get_index(node)?;
        self.members[index] = RegistryMember::new(node, key);
        Ok(index)
    }

    /// Remove a node: the last member takes its place.
    pub fn swap_remove_node(&mut self, node: Pubkey) -> Result<()> {
        let index = self.get_index(node)?;
        self.swap_remove_index(index);
        Ok(())
    }

    /// Remove every listed node that is a member, skipping the ones that are not.
    pub fn swap_remove_nodes(&mut self, remove_nodes: &[Pubkey]) -> Result<()> {
        for remove_node in remove_nodes {
            if let Some(index) = index_of(self.populated(), *remove_node) {
                self.swap_remove_index(index);
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

    fn swap_remove_index(&mut self, index: usize) {
        let last_index = usize::from(self.header.node_count) - 1;
        self.members[index] = self.members[last_index];
        self.members[last_index] = RegistryMember::new(Pubkey::default(), ([0; 32], [0; 32]));
        self.header.node_count -= 1;
    }
}

const _: () = assert!(RegistryMember::LEN == NodeEntry::LEN);
const _: () = assert!(Registry::HEADER_SPACE == 32);
// One instruction can create an account of up to 10 KiB, which is a snapshot of 106 members.
// A larger one is grown first by `extend_registry`.
const _: () = assert!(Registry::space(106) <= 10_240 && Registry::space(107) > 10_240);
const _: () = assert!(Registry::space(MAX_REGISTRY_NODES) == 24_608);
