use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{Bitmap, NonceEquationError};

use crate::error::{map_attestation_err, MolphaError};
use crate::state::{Evidence, EvidenceAccounts, Node, MAX_REGISTRY_NODES};
use crate::utils::slash::canonical_pair;

pub const DOMAIN_NONCE_REUSE: &[u8] = b"MOLPHA_NONCE_REUSE_V1";
pub const DOMAIN_COMMITMENT_BATCH: &[u8] = b"MOLPHA_COMMITMENT_BATCH_V1";

/// Prefixes separating leaf hashes from internal-node hashes, so no internal node can be replayed
/// as a leaf.
pub const COMMITMENT_MERKLE_LEAF_PREFIX: &[u8] = &[0x00];
pub const COMMITMENT_MERKLE_NODE_PREFIX: &[u8] = &[0x01];

/// Deepest accepted merkle path. A depth-14 tree addresses 16_384 slots per batch, and the filing
/// still fits a v0 transaction with an address lookup table.
pub const MAX_COMMITMENT_PROOF_DEPTH: usize = 14;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct NonceReuseEvidence {
    /// Registry version that `node_index` indexes into; also bound into the batch signature.
    pub registry_version: u32,
    pub node_index: u32,
    pub epoch: u64,
    /// Opaque batch header bytes, covered by the signature but not interpreted on-chain.
    pub batch_meta: [u8; 32],
    pub batch_root: [u8; 32],
    /// Single-key EVM-Schnorr signature over the batch message.
    pub batch_sig_s: [u8; 32],
    pub batch_commitment_addr: [u8; 20],
    pub slot: u64,
    pub nonces: CommittedNoncePair,
    pub merkle_proof: Vec<[u8; 32]>,
    pub transcript_a: PartialTranscript,
    pub transcript_b: PartialTranscript,
}

impl Evidence for NonceReuseEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        require!(
            self.merkle_proof.len() <= MAX_COMMITMENT_PROOF_DEPTH,
            MolphaError::MerkleProofTooDeep
        );

        // The equation fixes `s` given `(b, c)`, so differing shares alone prove nothing; the reuse is
        // two distinct challenge pairs consumed against one commitment.
        require!(
            (self.transcript_a.b, self.transcript_a.c)
                != (self.transcript_b.b, self.transcript_b.c),
            MolphaError::TranscriptContextsEqual
        );

        let node_index = usize::try_from(self.node_index)
            .map_err(|_| error!(MolphaError::InvalidNodeIndex))?;
        require!(node_index < MAX_REGISTRY_NODES, MolphaError::InvalidNodeIndex);
        let mut guilty_bitmap = Bitmap::new();
        guilty_bitmap.set_bit(node_index);

        require!(ctx.remaining.len() == 1, MolphaError::InvalidNodeAccount);
        let node: Account<Node> = Account::try_from(&ctx.remaining[0])?;
        let registered_node = ctx.evidence_registry.load()?.node(node_index)?;
        require!(node.key() == registered_node, MolphaError::InvalidNodeAccount);

        let leaf = commitment_leaf_hash(self.slot, &self.nonces);
        require!(
            verify_commitment_merkle_proof(leaf, &self.merkle_proof, &self.batch_root),
            MolphaError::InvalidMerkleProof
        );

        let batch_message = commitment_batch_message_hash(
            &crate::ID,
            self.registry_version,
            self.node_index,
            self.epoch,
            &self.batch_meta,
            &self.batch_root,
        );
        require!(
            molpha_verifier::verify_aggregate_over_hash(
                &self.batch_sig_s,
                &self.batch_commitment_addr,
                &batch_message,
                &[(node.secp256k1_pubkey_x, node.secp256k1_pubkey_y)],
            )
            .map_err(map_attestation_err)?,
            MolphaError::InvalidBatchSignature
        );

        let r1 = (self.nonces.r1_x, self.nonces.r1_y);
        let r2 = (self.nonces.r2_x, self.nonces.r2_y);
        for transcript in [&self.transcript_a, &self.transcript_b] {
            require!(
                molpha_verifier::verify_partial_signature_equation(
                    &(node.secp256k1_pubkey_x, node.secp256k1_pubkey_y),
                    &r1,
                    &r2,
                    &transcript.b,
                    &transcript.c,
                    &transcript.s,
                )
                .map_err(map_nonce_equation_err)?,
                MolphaError::InvalidPartialSignature
            );
        }

        Ok((guilty_bitmap, guilty_bitmap))
    }

    fn hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(self.transcript_a.hash(), self.transcript_b.hash());
        hashv(&[
            DOMAIN_NONCE_REUSE,
            self.registry_version.to_be_bytes().as_slice(),
            self.node_index.to_be_bytes().as_slice(),
            self.epoch.to_be_bytes().as_slice(),
            self.batch_meta.as_slice(),
            self.batch_root.as_slice(),
            self.slot.to_be_bytes().as_slice(),
            self.nonces.r1_x.as_slice(),
            self.nonces.r1_y.as_slice(),
            self.nonces.r2_x.as_slice(),
            self.nonces.r2_y.as_slice(),
            lo.as_slice(),
            hi.as_slice(),
        ])
        .to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.registry_version
    }
}

/// One partial-signature transcript: the challenge scalars and the resulting share.
///
/// The program never recomputes `b` or `c` from a signing context — the offence is defined purely
/// by the equation `s*G == R1 + b*R2 + c*X`, which is why message transport and commitment lists
/// stay off-chain.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartialTranscript {
    /// Binding factor.
    pub b: [u8; 32],
    /// Challenge.
    pub c: [u8; 32],
    /// The node's partial signature scalar.
    pub s: [u8; 32],
}

impl PartialTranscript {
    pub fn hash(&self) -> [u8; 32] {
        hashv(&[self.b.as_slice(), self.c.as_slice(), self.s.as_slice()]).to_bytes()
    }
}

fn map_nonce_equation_err(error: NonceEquationError) -> anchor_lang::error::Error {
    match error {
        NonceEquationError::InvalidNoncePoint => error!(MolphaError::InvalidNoncePoint),
        NonceEquationError::NonCanonicalScalar => error!(MolphaError::NonCanonicalScalar),
    }
}

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
