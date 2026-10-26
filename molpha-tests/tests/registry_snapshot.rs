//! Registry snapshot mechanics: strict version succession, `copy_from_previous` /
//! `swap_remove_node(s)` isolation, grace expiry via `is_active`, the rescan that survives the
//! index shift `swap_remove` causes, the signing keys a snapshot carries, and how a snapshot is
//! read from and written to its account.
//!
//! Pure unit tests — no VM.

use anchor_lang::prelude::{AccountInfo, Pubkey};
use anchor_lang::Discriminator;
use molpha::state::{
    Registry, RegistryBuilder, RegistryMember, RegistryMut, RegistryRef, MAX_REGISTRY_NODES,
};

type Key = ([u8; 32], [u8; 32]);

fn key(seed: u8) -> Key {
    ([seed; 32], [seed.wrapping_add(0x80); 32])
}

const NO_MEMBER: RegistryMember = RegistryMember {
    node: [0; 32],
    x: [0; 32],
    y: [0; 32],
};

/// A snapshot held in memory: a header, and a member table with room for `members.len()`.
struct Snapshot {
    header: Registry,
    members: Vec<RegistryMember>,
}

impl Snapshot {
    fn with_room(room: usize) -> Self {
        Self {
            header: Registry {
                grace_active_until: 0,
                active_from: 0,
                version: 0,
                node_count: 0,
                redundancy_buffer: 0,
                bump: 0,
            },
            members: vec![NO_MEMBER; room],
        }
    }

    /// A snapshot whose members are `nodes`, each with a distinct key, and no room to spare.
    fn of(nodes: &[Pubkey]) -> Self {
        let mut snapshot = Self::with_room(nodes.len());
        for (i, node) in nodes.iter().enumerate() {
            snapshot
                .builder()
                .append_node(*node, key(i as u8 + 1))
                .unwrap();
        }
        snapshot
    }

    fn builder(&mut self) -> RegistryBuilder<'_> {
        RegistryBuilder {
            header: &mut self.header,
            members: &mut self.members,
        }
    }

    fn populated(&self) -> &[RegistryMember] {
        &self.members[..usize::from(self.header.node_count)]
    }

    fn node(&self, index: usize) -> Pubkey {
        Pubkey::new_from_array(self.populated()[index].node)
    }

    fn key(&self, index: usize) -> Key {
        self.populated()[index].xy()
    }

    /// The next version: a copy of this one with room for `room` members.
    fn successor(&mut self, room: usize) -> Snapshot {
        let mut next = Snapshot::with_room(room);
        let count = usize::from(self.header.node_count);
        next.builder()
            .copy_from_previous(&mut self.header, &self.members[..count], 1, 1, 0)
            .unwrap();
        next
    }
}

fn nodes(count: usize) -> Vec<Pubkey> {
    (0..count).map(|_| Pubkey::new_unique()).collect()
}

#[test]
fn snapshot_copy_and_swap_remove_do_not_mutate_source() {
    let members = nodes(3);
    let mut source = Snapshot::of(&members);
    source.header.version = 7;
    source.header.redundancy_buffer = 2;
    source.header.bump = 1;

    let mut next = Snapshot::with_room(3);
    next.builder()
        .copy_from_previous(&mut source.header, &source.members, 8, 3, 2)
        .unwrap();
    next.builder().swap_remove_node(members[1]).unwrap();

    assert_eq!(source.header.node_count, 3);
    assert_eq!(source.node(1), members[1]);
    assert_eq!(source.header.grace_active_until, 8);
    assert_eq!(next.header.version, 8);
    assert_eq!(next.header.active_from, 3);
    assert_eq!(next.header.grace_active_until, 0);
    assert_eq!(next.header.redundancy_buffer, 2);
    assert_eq!(next.header.bump, 2);
    assert_eq!(next.header.node_count, 2);
    assert_eq!(next.node(1), members[2]);
    // The slot the removal freed is cleared, so shrinking the account loses nothing.
    assert_eq!(next.members[2].node, [0; 32]);
    assert_eq!(next.members[2].xy(), ([0; 32], [0; 32]));
}

#[test]
fn swap_remove_batch_handles_swap_index_hazard() {
    let members = nodes(4);
    let mut current = Snapshot::of(&members);
    let mut next = current.successor(4);

    next.builder()
        .swap_remove_nodes(&[members[0], members[3]])
        .unwrap();
    assert_eq!(next.header.node_count, 2);
    let remaining: Vec<Pubkey> = (0..2).map(|i| next.node(i)).collect();
    assert!(remaining.contains(&members[1]));
    assert!(remaining.contains(&members[2]));
    assert_eq!(current.header.node_count, 4);
}

#[test]
fn swap_remove_batch_skips_absent_victim() {
    let mut current = Snapshot::of(&nodes(1));
    let mut next = current.successor(1);

    next.builder()
        .swap_remove_nodes(&[Pubkey::new_unique()])
        .unwrap();
    assert_eq!(next.header.node_count, 1);
}

#[test]
fn registry_is_active_until_grace_expires() {
    let registry = Snapshot::with_room(0).header;
    assert!(registry.is_active(0));
    assert!(registry.is_active(i64::MAX));

    let mut expired = Snapshot::with_room(0).header;
    expired.grace_active_until = 100;
    assert!(expired.is_active(99));
    assert!(!expired.is_active(100));
    assert!(!expired.is_active(101));

    expired.active_from = 20;
    assert!(!expired.is_active(19));
    assert!(expired.is_active(20));
}

#[test]
fn members_carry_their_keys() {
    let members = nodes(3);
    let snapshot = Snapshot::of(&members);
    for (i, node) in members.iter().enumerate() {
        assert_eq!(snapshot.node(i), *node);
        assert_eq!(snapshot.key(i), key(i as u8 + 1));
    }
}

#[test]
fn swap_remove_moves_a_members_key_with_it() {
    let members = nodes(3);
    let mut current = Snapshot::of(&members);
    let mut next = current.successor(3);
    next.builder().swap_remove_node(members[0]).unwrap();

    // The last member fills the hole, carrying its own key rather than the departed one's.
    assert_eq!(next.node(0), members[2]);
    assert_eq!(next.key(0), key(3));
    assert_eq!(next.key(1), key(2));
}

#[test]
fn replace_key_changes_one_member_and_keeps_its_index() {
    let members = nodes(3);
    let mut current = Snapshot::of(&members);
    let mut next = current.successor(3);

    let index = next.builder().replace_key(members[1], key(0x40)).unwrap();

    assert_eq!(index, 1);
    assert_eq!(next.header.node_count, 3);
    assert_eq!(next.node(1), members[1]);
    assert_eq!(next.key(1), key(0x40));
    // Neighbours are untouched, and the version it was copied from still names the old key.
    assert_eq!(next.key(0), key(1));
    assert_eq!(next.key(2), key(3));
    assert_eq!(current.key(1), key(2));
}

#[test]
fn replace_key_rejects_a_node_that_is_not_a_member() {
    let mut snapshot = Snapshot::of(&nodes(1));
    assert!(snapshot
        .builder()
        .replace_key(Pubkey::new_unique(), key(0x40))
        .is_err());
}

#[test]
fn a_node_cannot_appear_twice() {
    let members = nodes(1);
    let mut current = Snapshot::of(&members);
    let mut next = current.successor(2);
    assert!(next.builder().append_node(members[0], key(9)).is_err());
}

#[test]
fn appending_needs_room_in_the_table() {
    let members = nodes(2);
    let mut current = Snapshot::of(&members);

    // A successor sized for one more member takes it; one sized for the same count does not.
    let mut roomy = current.successor(3);
    roomy
        .builder()
        .append_node(Pubkey::new_unique(), key(7))
        .unwrap();
    assert_eq!(roomy.header.node_count, 3);

    let mut tight = current.successor(2);
    assert!(tight
        .builder()
        .append_node(Pubkey::new_unique(), key(7))
        .is_err());
}

#[test]
fn copying_needs_room_for_every_member() {
    let mut current = Snapshot::of(&nodes(3));
    let mut small = Snapshot::with_room(2);
    assert!(small
        .builder()
        .copy_from_previous(&mut current.header, &current.members, 1, 1, 0)
        .is_err());
    // And the member slice must be the previous snapshot's own members.
    let mut next = Snapshot::with_room(3);
    assert!(next
        .builder()
        .copy_from_previous(&mut current.header, &current.members[..2], 1, 1, 0)
        .is_err());
}

#[test]
fn a_registry_holds_at_most_one_member_per_bitmap_bit() {
    // More room than a 256-bit signer bitmap can address.
    let mut snapshot = Snapshot::with_room(MAX_REGISTRY_NODES + 8);
    for i in 0..MAX_REGISTRY_NODES {
        snapshot
            .builder()
            .append_node(Pubkey::new_unique(), key(i as u8))
            .unwrap();
    }
    assert_eq!(usize::from(snapshot.header.node_count), MAX_REGISTRY_NODES);
    assert_eq!(MAX_REGISTRY_NODES, 256);
    assert!(
        snapshot
            .builder()
            .append_node(Pubkey::new_unique(), key(0xee))
            .is_err(),
        "the 257th member has no bit"
    );
}

// ---------------------------------------------------------------------------
// reading and writing a snapshot through its account
// ---------------------------------------------------------------------------

/// Owned state behind a registry `AccountInfo`. The data is backed by `u64`s so the header's
/// alignment is guaranteed, as it is for account data in the VM.
struct RegistryAccount {
    key: Pubkey,
    owner: Pubkey,
    lamports: u64,
    words: Vec<u64>,
    len: usize,
}

impl RegistryAccount {
    /// An allocated, never-written account of `len` bytes.
    fn allocated(len: usize) -> Self {
        Self {
            key: Pubkey::new_unique(),
            owner: molpha::ID,
            lamports: 1,
            words: vec![0u64; len.div_ceil(8)],
            len,
        }
    }

    fn info(&mut self) -> AccountInfo<'_> {
        let bytes: &mut [u8] = anchor_lang::__private::bytemuck::cast_slice_mut(&mut self.words);
        AccountInfo::new(
            &self.key,
            false,
            true,
            &mut self.lamports,
            &mut bytes[..self.len],
            &self.owner,
            false,
        )
    }

    fn bytes(&mut self) -> &mut [u8] {
        let bytes: &mut [u8] = anchor_lang::__private::bytemuck::cast_slice_mut(&mut self.words);
        &mut bytes[..self.len]
    }

    /// An initialised snapshot of `nodes`, written through `RegistryMut`.
    fn of(nodes: &[Pubkey]) -> Self {
        let mut account = Self::allocated(Registry::space(nodes.len()));
        {
            let info = account.info();
            let mut registry = RegistryMut::load_init(&info).unwrap();
            let mut builder = registry.builder();
            builder.header.version = 5;
            builder.header.redundancy_buffer = 2;
            for (i, node) in nodes.iter().enumerate() {
                builder.append_node(*node, key(i as u8 + 1)).unwrap();
            }
        }
        account
    }
}

#[test]
fn a_snapshot_written_to_an_account_reads_back_with_its_members() {
    let members = nodes(3);
    let mut account = RegistryAccount::of(&members);
    assert_eq!(account.bytes()[..8], *Registry::DISCRIMINATOR);

    let info = account.info();
    let registry = RegistryRef::load(&info).unwrap();
    assert_eq!(registry.version, 5);
    assert_eq!(registry.node_count, 3);
    assert_eq!(registry.members().len(), 3);
    for (i, node) in members.iter().enumerate() {
        assert_eq!(registry.node(i).unwrap(), *node);
        assert_eq!(registry.member(i).unwrap().xy(), key(i as u8 + 1));
        assert_eq!(registry.get_index(*node).unwrap(), i);
    }
    assert!(registry.member(3).is_err(), "only members are readable");
    assert!(registry.is_node_in_registry(members[2]));
    assert!(!registry.is_node_in_registry(Pubkey::new_unique()));
    assert!(registry.get_index(Pubkey::new_unique()).is_err());

    // The verifier sees the same table in place.
    let view = registry.view();
    assert_eq!(view.version, 5);
    assert_eq!(view.node_count, 3);
    assert_eq!(view.redundancy_buffer, 2);
    assert_eq!(view.nodes.len(), 3);
    assert_eq!(view.nodes[2].account, members[2].to_bytes());
    assert_eq!(view.nodes[2].xy(), key(3));
    // Selection needs no members.
    assert!(registry.selection_view().nodes.is_empty());
    assert_eq!(registry.selection_view().node_count, 3);
}

#[test]
fn an_account_is_initialised_as_a_registry_only_once() {
    let mut account = RegistryAccount::allocated(Registry::space(2));
    {
        let info = account.info();
        RegistryMut::load_init(&info).unwrap();
    }
    let info = account.info();
    assert!(RegistryMut::load_init(&info).is_err());
    assert!(RegistryMut::load(&info).is_ok());
}

#[test]
fn an_unwritten_account_is_not_a_registry() {
    let mut account = RegistryAccount::allocated(Registry::space(2));
    let info = account.info();
    assert!(RegistryRef::load(&info).is_err());
    assert!(RegistryMut::load(&info).is_err());
}

#[test]
fn a_registry_must_be_owned_by_the_program() {
    let mut account = RegistryAccount::of(&nodes(1));
    account.owner = Pubkey::new_unique();
    let info = account.info();
    assert!(RegistryRef::load(&info).is_err());
    assert!(RegistryMut::load(&info).is_err());
    assert!(RegistryMut::load_init(&info).is_err());
}

#[test]
fn a_registry_account_is_a_header_and_whole_member_records() {
    // Too short for a header, and a table that ends mid-record.
    for len in [
        0,
        8,
        Registry::HEADER_SPACE - 1,
        Registry::space(1) - 1,
        Registry::space(1) + 1,
    ] {
        let mut account = RegistryAccount::allocated(len);
        let info = account.info();
        assert!(RegistryMut::load_init(&info).is_err(), "length {len}");
    }

    // A header that claims more members than the account holds.
    let mut account = RegistryAccount::of(&nodes(2));
    account.bytes()[8 + 20..8 + 22].copy_from_slice(&3u16.to_le_bytes());
    let info = account.info();
    assert!(RegistryRef::load(&info).is_err());
}
