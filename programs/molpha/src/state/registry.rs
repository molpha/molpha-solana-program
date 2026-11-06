use core::cell::{Ref, RefMut};
use core::ops::Deref;

use anchor_lang::prelude::*;
use anchor_lang::Discriminator;
pub use molpha_verifier::{Bitmap, MAX_PROVIDER_SLOTS, NODE_SLOTS, PROVIDER_BIT_BASE};
use molpha_verifier::{NodeEntry, RegistryView, SignerXy};

use crate::error::MolphaError;

/// Width of a signer bitmap, and so the most records a registry can hold: the node slots
/// `[0, NODE_SLOTS)` and the provider slots `[NODE_SLOTS, 256)` together.
pub const MAX_REGISTRY_NODES: usize = 256;

/// Provider slot 0 (bit 64) is reserved for the Molpha signer and stays empty in this build, so
/// registration takes the lowest free provider slot from here.
pub const FIRST_PROVIDER_SLOT: usize = 1;

/// The node bits of a signers bitmap: `[0, NODE_SLOTS)`. Selection, credit, tickets, epoch rosters
/// and slashing see only these; a provider bit counts toward the quorum and proves provenance.
pub const NODE_MASK: Bitmap = Bitmap::NODE_MASK;

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
/// many slots each segment has.
///
/// The members follow the header in the same account as two tables, one [`RegistryMember`] each:
/// the `node_count` node slots (node slot `i` signs at bit `i`), then the `provider_count` provider
/// slots (provider slot `p` signs at bit `PROVIDER_BIT_BASE + p`). A provider therefore costs one
/// record, not padding up to bit 64.
///
/// A member keeps its slot for as long as it is in a snapshot: removing one leaves a hole (a vacant
/// record) rather than moving the last member into it, and a later registration takes the lowest
/// hole of its segment. Each count includes the holes; trailing holes are trimmed, so the last
/// slot of each segment is live. The account is sized to hold exactly the two tables
/// ([`Registry::space`] of [`Registry::records`]), so a snapshot costs what its slots need. That is
/// also how the verifier reads it. Use [`RegistryRef`] to read a snapshot with its members; this
/// struct alone gives the header.
///
/// A snapshot without providers has the bytes of the earlier layout, whose `node_count` was a
/// `u16` at the same offset.
#[account(zero_copy)]
#[repr(C)]
pub struct Registry {
    pub grace_active_until: i64,
    /// Inclusive creation time. A version cannot verify a round dated before it existed.
    pub active_from: i64,
    pub version: u32,
    /// Node slots, holes included: at most [`NODE_SLOTS`].
    pub node_count: u8,
    /// Provider slots, holes included: at most [`MAX_PROVIDER_SLOTS`].
    pub provider_count: u8,
    pub redundancy_buffer: u8,
    pub bump: u8,
}

impl Registry {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    /// Bytes before the member table: the Anchor discriminator and this header.
    pub const HEADER_SPACE: usize = 8 + core::mem::size_of::<Self>();

    /// Account size of a snapshot with room for `members` records, nodes and providers together.
    pub const fn space(members: usize) -> usize {
        Self::HEADER_SPACE + members * RegistryMember::LEN
    }

    /// Records an account of `data_len` bytes has room for, or `None` when that length is not a
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

    /// Records the snapshot stores: its node slots and its provider slots.
    pub fn records(&self) -> usize {
        usize::from(self.node_count) + usize::from(self.provider_count)
    }

    /// Whether the counts are within their segments and the table of `capacity` records holds both.
    fn counts_fit(&self, capacity: usize) -> bool {
        usize::from(self.node_count) <= NODE_SLOTS
            && usize::from(self.provider_count) <= MAX_PROVIDER_SLOTS
            && self.records() <= capacity
    }

    /// The signer bit of the record at table `index`.
    fn bit_of_record(&self, index: usize) -> usize {
        let nodes = usize::from(self.node_count);
        if index < nodes {
            index
        } else {
            PROVIDER_BIT_BASE + (index - nodes)
        }
    }

    /// The table index of signer bit `bit`, or `None` when its segment has no such slot.
    fn record_of_bit(&self, bit: usize) -> Option<usize> {
        if bit < NODE_SLOTS {
            (bit < usize::from(self.node_count)).then_some(bit)
        } else {
            let slot = bit - PROVIDER_BIT_BASE;
            (slot < usize::from(self.provider_count)).then(|| usize::from(self.node_count) + slot)
        }
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

/// Table index of `account` among `records`. A hole never matches, not even the default pubkey.
fn index_of(records: &[RegistryMember], account: Pubkey) -> Option<usize> {
    let account = account.to_bytes();
    if account == [0u8; 32] {
        return None;
    }
    records.iter().position(|member| member.node == account)
}

/// The lowest provider slot (from `FIRST_PROVIDER_SLOT`) free in a provider table: a vacant
/// record, or one past the end.
fn first_free_provider_slot(providers: &[RegistryMember]) -> Option<usize> {
    (FIRST_PROVIDER_SLOT..MAX_PROVIDER_SLOTS)
        .find(|slot| providers.get(*slot).is_none_or(RegistryMember::is_vacant))
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
/// Derefs to the header. Every accessor that takes or returns an index speaks signer bits: node
/// slot `i` is bit `i`, provider slot `p` is bit `PROVIDER_BIT_BASE + p`.
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
            bytemuck::from_bytes::<Registry>(header).counts_fit(table.len() / RegistryMember::LEN)
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

    /// Every stored record: the node table, then the provider table.
    pub fn records(&self) -> &[RegistryMember] {
        &self.table[..self.header.records()]
    }

    /// The node slots in order. A removed node's slot is a vacant record.
    pub fn nodes(&self) -> &[RegistryMember] {
        &self.table[..usize::from(self.header.node_count)]
    }

    /// The provider slots in order: provider slot `p` is `providers()[p]`.
    pub fn providers(&self) -> &[RegistryMember] {
        &self.table[usize::from(self.header.node_count)..self.header.records()]
    }

    /// The live member signing at `bit`; a hole or a slot past its segment is an invalid index.
    pub fn member(&self, bit: usize) -> Result<RegistryMember> {
        self.header
            .record_of_bit(bit)
            .map(|index| self.table[index])
            .filter(|member| !member.is_vacant())
            .ok_or(error!(MolphaError::InvalidNodeIndex))
    }

    /// How many records hold a live member, nodes and providers together.
    pub fn live_count(&self) -> usize {
        self.records().iter().filter(|m| !m.is_vacant()).count()
    }

    /// How many node slots hold a live node.
    pub fn live_node_count(&self) -> usize {
        self.nodes().iter().filter(|m| !m.is_vacant()).count()
    }

    pub fn node(&self, bit: usize) -> Result<Pubkey> {
        Ok(Pubkey::new_from_array(self.member(bit)?.node))
    }

    /// The signer bit of `account`.
    pub fn get_index(&self, account: Pubkey) -> Result<usize> {
        index_of(self.records(), account)
            .map(|index| self.header.bit_of_record(index))
            .ok_or(error!(MolphaError::InvalidNodeAccount))
    }

    pub fn is_node_in_registry(&self, account: Pubkey) -> bool {
        index_of(self.records(), account).is_some()
    }

    /// The lowest provider slot a registration would take, or `None` when all are taken.
    pub fn first_free_provider_slot(&self) -> Option<usize> {
        first_free_provider_slot(self.providers())
    }

    /// Framework-agnostic view for [`molpha_verifier`], with both tables viewed in place.
    /// Selection needs the node table too: it draws over the live nodes and skips the holes.
    pub fn view(&self) -> RegistryView<'_> {
        fn entries(members: &[RegistryMember]) -> &[NodeEntry] {
            NodeEntry::slice_from_bytes(bytemuck::cast_slice(members))
                .expect("a member record is one verifier NodeEntry")
        }
        RegistryView {
            version: self.header.version,
            node_count: u16::from(self.header.node_count),
            provider_count: u16::from(self.header.provider_count),
            redundancy_buffer: self.header.redundancy_buffer,
            nodes: entries(self.nodes()),
            providers: entries(self.providers()),
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
            header.counts_fit(table.len()),
            ErrorCode::AccountDidNotDeserialize
        );
        Ok(Self { header, table })
    }

    /// The stored records: the node table, then the provider table.
    pub fn records(&self) -> &[RegistryMember] {
        &self.table[..self.header.records()]
    }

    /// The header for writing, together with the stored records.
    pub fn parts(&mut self) -> (&mut Registry, &[RegistryMember]) {
        let count = self.header.records();
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
/// account has. Every operation keeps `members[..node_count]` as the node slots,
/// `members[node_count..records()]` as the provider slots (each segment's last slot live) and the
/// rest zeroed. Adding or trimming a node slot moves the provider table with it.
pub struct RegistryBuilder<'a> {
    pub header: &'a mut Registry,
    pub members: &'a mut [RegistryMember],
}

impl RegistryBuilder<'_> {
    fn populated(&self) -> &[RegistryMember] {
        &self.members[..self.header.records()]
    }

    fn node_count(&self) -> usize {
        usize::from(self.header.node_count)
    }

    fn provider_count(&self) -> usize {
        usize::from(self.header.provider_count)
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
            previous_members.len() == previous.records()
                && previous_members.len() <= self.members.len(),
            MolphaError::NodeCapExceeded
        );
        self.header.version = previous.version + 1;
        self.header.node_count = previous.node_count;
        self.header.provider_count = previous.provider_count;
        self.header.redundancy_buffer = previous.redundancy_buffer;
        self.header.bump = bump;
        self.header.active_from = active_from;
        self.header.grace_active_until = 0;
        self.members[..previous_members.len()].copy_from_slice(previous_members);
        previous.grace_active_until = grace_active_until;
        Ok(())
    }

    /// Add a node and its key in the lowest vacant node slot, or after the last node slot when
    /// there is none (moving the provider table one record on). Returns the slot, which is the
    /// node's bit. A registry holds at most `NODE_SLOTS` nodes.
    pub fn insert_node(&mut self, node: Pubkey, key: SignerXy) -> Result<usize> {
        require!(
            !self.is_node_in_registry(node),
            MolphaError::NodeAlreadyInRegistry
        );
        let nodes = self.node_count();
        if let Some(slot) = self.members[..nodes]
            .iter()
            .position(RegistryMember::is_vacant)
        {
            self.members[slot] = RegistryMember::new(node, key);
            return Ok(slot);
        }
        let records = self.header.records();
        require!(
            nodes < NODE_SLOTS && records < self.members.len(),
            MolphaError::NodeCapExceeded
        );
        self.members.copy_within(nodes..records, nodes + 1);
        self.members[nodes] = RegistryMember::new(node, key);
        // `nodes` is below `NODE_SLOTS`, so the new count fits.
        self.header.node_count = (nodes + 1) as u8;
        Ok(nodes)
    }

    /// The lowest provider slot (from `FIRST_PROVIDER_SLOT`) whose record is vacant, or past the
    /// last provider slot. `None` when all `MAX_PROVIDER_SLOTS` are taken.
    pub fn first_free_provider_slot(&self) -> Option<usize> {
        first_free_provider_slot(&self.populated()[self.node_count()..])
    }

    /// Add a provider, whose registry record is its `Provider` PDA and its signing key, in the
    /// lowest free provider slot. Returns the slot. Slots between the old last provider slot and
    /// the new one (slot 0, the first time) are holes.
    pub fn insert_provider(&mut self, provider: Pubkey, key: SignerXy) -> Result<usize> {
        require!(
            !self.is_node_in_registry(provider),
            MolphaError::ProviderAlreadyInRegistry
        );
        let slot = self
            .first_free_provider_slot()
            .ok_or(error!(MolphaError::ProviderSlotsFull))?;
        let base = self.node_count();
        let providers = self.provider_count();
        if slot >= providers {
            require!(
                base + slot < self.members.len(),
                MolphaError::NodeCapExceeded
            );
            // The records past the stored ones are zeroed: the new holes are already vacant.
            // `slot` is below `MAX_PROVIDER_SLOTS`, so the new count fits.
            self.header.provider_count = (slot + 1) as u8;
        }
        self.members[base + slot] = RegistryMember::new(provider, key);
        Ok(slot)
    }

    /// Replace a member's key. The member keeps its slot. Returns its signer bit.
    pub fn replace_key(&mut self, account: Pubkey, key: SignerXy) -> Result<usize> {
        let index = self.record_index(account)?;
        self.members[index] = RegistryMember::new(account, key);
        Ok(self.header.bit_of_record(index))
    }

    /// Remove a node, leaving its slot vacant.
    pub fn remove_node(&mut self, node: Pubkey) -> Result<()> {
        let index = self.record_index(node)?;
        require!(index < self.node_count(), MolphaError::InvalidNodeAccount);
        self.vacate_node(index);
        Ok(())
    }

    /// Remove a provider, leaving its record vacant. Returns the provider slot it held.
    pub fn remove_provider(&mut self, provider: Pubkey) -> Result<usize> {
        let index = self.record_index(provider)?;
        let base = self.node_count();
        require!(index >= base, MolphaError::ProviderNotInRegistry);
        let slot = index - base;
        self.members[index] = RegistryMember::VACANT;
        let mut count = self.provider_count();
        while count > 0 && self.members[base + count - 1].is_vacant() {
            count -= 1;
        }
        self.header.provider_count = count as u8;
        Ok(slot)
    }

    /// Remove every listed node that is a member, skipping the ones that are not. Every other
    /// member keeps its slot. Providers are never removed this way.
    pub fn remove_nodes(&mut self, remove_nodes: &[Pubkey]) -> Result<()> {
        for remove_node in remove_nodes {
            if let Some(index) = index_of(self.populated(), *remove_node) {
                require!(index < self.node_count(), MolphaError::InvalidNodeAccount);
                self.vacate_node(index);
            }
        }
        Ok(())
    }

    /// The signer bit of `account`.
    pub fn get_index(&self, account: Pubkey) -> Result<usize> {
        Ok(self.header.bit_of_record(self.record_index(account)?))
    }

    pub fn is_node_in_registry(&self, account: Pubkey) -> bool {
        index_of(self.populated(), account).is_some()
    }

    fn record_index(&self, account: Pubkey) -> Result<usize> {
        index_of(self.populated(), account).ok_or(error!(MolphaError::InvalidNodeAccount))
    }

    /// Empty a node slot, then drop the trailing node holes, moving the provider table down to
    /// follow the last live node and zeroing the records it leaves.
    fn vacate_node(&mut self, index: usize) {
        self.members[index] = RegistryMember::VACANT;
        let nodes = self.node_count();
        let records = self.header.records();
        let mut count = nodes;
        while count > 0 && self.members[count - 1].is_vacant() {
            count -= 1;
        }
        if count < nodes {
            self.members.copy_within(nodes..records, count);
            let freed = records - (nodes - count);
            self.members[freed..records].fill(RegistryMember::VACANT);
            self.header.node_count = count as u8;
        }
    }
}

const _: () = assert!(RegistryMember::LEN == NodeEntry::LEN);
const _: () = assert!(Registry::HEADER_SPACE == 32);
// One instruction can create an account of up to 10 KiB, which is a snapshot of 106 members.
// A larger one is grown first by `extend_registry`.
const _: () = assert!(Registry::space(106) <= 10_240 && Registry::space(107) > 10_240);
const _: () = assert!(Registry::space(MAX_REGISTRY_NODES) == 24_608);
