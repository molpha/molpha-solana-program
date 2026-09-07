//! Signed nonce-commitment batches: message binding, merkle leaves, and proof verification.
//!
//! A node commits its per-slot nonce pairs `(epoch, slot) -> (R1, R2)` as merkle leaves and signs
//! the root once. Slashing evidence carries one leaf plus its path, so the chain never sees the
//! rest of the batch. `batch_meta` is opaque to the program: it is covered by the signature and
//! by the evidence hash, but nothing on-chain interprets it.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const DOMAIN_COMMITMENT_BATCH: &[u8] = b"MOLPHA_COMMITMENT_BATCH_V1";

/// Prefixes separating leaf hashes from internal-node hashes, so no internal node can be replayed
/// as a leaf.
pub const COMMITMENT_MERKLE_LEAF_PREFIX: &[u8] = &[0x00];
pub const COMMITMENT_MERKLE_NODE_PREFIX: &[u8] = &[0x01];

/// Deepest accepted merkle path. A depth-14 tree addresses 16_384 slots per batch, and the filing
/// still fits a v0 transaction with an address lookup table.
pub const MAX_COMMITMENT_PROOF_DEPTH: usize = 14;

/// The nonce pair committed for one slot, as affine coordinates.
///
/// Points travel uncompressed so the program never pays for a field square root; both are
/// re-checked on-curve during equation verification.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedNoncePair {
    pub r1_x: [u8; 32],
    pub r1_y: [u8; 32],
    pub r2_x: [u8; 32],
    pub r2_y: [u8; 32],
}

/// Message a node signs to commit a batch root.
///
/// `program_id` is bound so a batch signed against one deployment cannot be replayed as evidence
/// on another, matching the node proof-of-possession. `registry_version` is bound because
/// `node_index` only identifies a node within one registry version.
pub fn commitment_batch_message_hash(
    program_id: &Pubkey,
    registry_version: u32,
    node_index: u32,
    epoch: u64,
    batch_meta: &[u8; 32],
    batch_root: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        DOMAIN_COMMITMENT_BATCH,
        program_id.as_ref(),
        registry_version.to_be_bytes().as_slice(),
        node_index.to_be_bytes().as_slice(),
        epoch.to_be_bytes().as_slice(),
        batch_meta.as_slice(),
        batch_root.as_slice(),
    ])
    .to_bytes()
}

/// Merkle leaf committing one slot's nonce pair.
pub fn commitment_leaf_hash(slot: u64, nonces: &CommittedNoncePair) -> [u8; 32] {
    hashv(&[
        COMMITMENT_MERKLE_LEAF_PREFIX,
        slot.to_be_bytes().as_slice(),
        nonces.r1_x.as_slice(),
        nonces.r1_y.as_slice(),
        nonces.r2_x.as_slice(),
        nonces.r2_y.as_slice(),
    ])
    .to_bytes()
}

/// Fold a leaf through its proof and compare against the signed root.
///
/// Siblings are ordered by value rather than by position, so the proof carries no direction bits.
pub fn verify_commitment_merkle_proof(leaf: [u8; 32], proof: &[[u8; 32]], root: &[u8; 32]) -> bool {
    let mut node = leaf;
    for sibling in proof {
        let (lo, hi) = if node <= *sibling {
            (node, *sibling)
        } else {
            (*sibling, node)
        };
        node = hashv(&[COMMITMENT_MERKLE_NODE_PREFIX, lo.as_slice(), hi.as_slice()]).to_bytes();
    }
    node == *root
}
