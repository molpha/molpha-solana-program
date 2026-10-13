//! secp256k1 test signer: node keys, proofs of possession, and coalition Schnorr signatures.
//!
//! The signing side of what `molpha_verifier` verifies. Kept deliberately independent of the
//! verifier's internals (it re-derives the challenge from the spec) so a change to either side
//! shows up as a failing test rather than as two copies of the same mistake agreeing.

use anchor_lang::prelude::Pubkey;
use libsecp256k1::{curve::Scalar, PublicKey, SecretKey};
use molpha_verifier::{eth_address_from_uncompressed_pubkey, KeyRotationProof, NODE_POP_PREFIX};
use solana_keccak_hasher::hashv;

/// One registry node's secp256k1 identity.
#[derive(Clone, Debug)]
pub struct NodeKey {
    pub secret: [u8; 32],
    pub compressed: [u8; 33],
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl NodeKey {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        let sk = SecretKey::parse(&secret).expect("secret in range");
        let pk = PublicKey::from_secret_key(&sk);
        let compressed = pk.serialize_compressed();
        let uncompressed = pk.serialize();
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(&uncompressed[1..33]);
        y.copy_from_slice(&uncompressed[33..65]);
        Self {
            secret,
            compressed,
            x,
            y,
        }
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey::parse_compressed(&self.compressed).expect("valid point")
    }
}

/// Deterministic node keys for a test: `secret_i = keccak(domain, i)`, retried on the
/// astronomically unlikely out-of-range draw.
pub fn node_keys(domain: &str, count: usize) -> Vec<NodeKey> {
    (0..count)
        .map(|i| {
            let mut attempt = 0u32;
            loop {
                let seed = hashv(&[
                    domain.as_bytes(),
                    &(i as u32).to_be_bytes(),
                    &attempt.to_be_bytes(),
                ])
                .to_bytes();
                if SecretKey::parse(&seed).is_ok() {
                    return NodeKey::from_secret(seed);
                }
                attempt += 1;
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// scalar helpers (mod n)
// ---------------------------------------------------------------------------

fn scalar_from_be(bytes: &[u8; 32]) -> Scalar {
    let mut s = Scalar::default();
    // `set_b32` reports overflow; the reduced value is what the on-chain code uses too.
    let _ = s.set_b32(bytes);
    s
}

pub fn add_mod_n(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    (scalar_from_be(a) + scalar_from_be(b)).b32()
}

pub fn mul_mod_n(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    (scalar_from_be(a) * scalar_from_be(b)).b32()
}

pub fn negate_mod_n(a: &[u8; 32]) -> [u8; 32] {
    (-scalar_from_be(a)).b32()
}

/// Reduce a 32-byte big-endian hash into `[0, n)`.
pub fn reduce_mod_n(bytes: &[u8; 32]) -> [u8; 32] {
    scalar_from_be(bytes).b32()
}

// ---------------------------------------------------------------------------
// node id + proof of possession
// ---------------------------------------------------------------------------

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";

pub fn derive_node_id(
    owner: &Pubkey,
    secp256k1_pubkey_compressed: &[u8; 33],
    registration_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        NODE_ID_PREFIX,
        owner.as_ref(),
        secp256k1_pubkey_compressed.as_ref(),
        registration_salt.as_ref(),
    ])
    .to_bytes()
}

/// Sign the proof of possession that `register_node` verifies.
///
/// `signing_secret` is separate from `key.secret` so tests can forge a PoP for a key the
/// signer does not own (the rogue-key case).
pub fn sign_pop_with(
    program_id: &Pubkey,
    node_id: &[u8; 32],
    public_key_compressed: &[u8; 33],
    signing_secret: &[u8; 32],
    nonce: &[u8; 32],
) -> ([u8; 32], [u8; 32]) {
    // The verifier reconstructs R from `r` assuming even parity, so the nonce is normalized here.
    let mut canonical_nonce = *nonce;
    let mut nonce_pubkey = NodeKey::from_secret(canonical_nonce).compressed;
    if nonce_pubkey[0] == 0x03 {
        canonical_nonce = negate_mod_n(&canonical_nonce);
        nonce_pubkey = NodeKey::from_secret(canonical_nonce).compressed;
    }

    let r: [u8; 32] = nonce_pubkey[1..33].try_into().unwrap();
    let public_key_x: [u8; 32] = public_key_compressed[1..33].try_into().unwrap();
    let parity = [public_key_compressed[0] & 1];
    let message = hashv(&[NODE_POP_PREFIX, program_id.as_ref(), node_id.as_ref()]).to_bytes();
    let challenge = reduce_mod_n(
        &hashv(&[
            public_key_x.as_ref(),
            parity.as_ref(),
            message.as_ref(),
            r.as_ref(),
        ])
        .to_bytes(),
    );
    let s = add_mod_n(&canonical_nonce, &mul_mod_n(&challenge, signing_secret));
    (r, s)
}

pub fn sign_pop(
    program_id: &Pubkey,
    node_id: &[u8; 32],
    key: &NodeKey,
    nonce: &[u8; 32],
) -> ([u8; 32], [u8; 32]) {
    sign_pop_with(program_id, node_id, &key.compressed, &key.secret, nonce)
}

// ---------------------------------------------------------------------------
// coalition aggregate Schnorr
// ---------------------------------------------------------------------------

/// An aggregate signature over one message hash.
#[derive(Clone, Copy, Debug)]
pub struct AggregateSignature {
    pub agg_sig_s: [u8; 32],
    /// Ethereum-style address of the aggregate nonce point `R = k*G`.
    pub commitment: [u8; 20],
    /// The nonce `k` actually used. Reuse it across two messages to forge nonce-reuse evidence.
    pub nonce: [u8; 32],
}

/// Sum of the coalition's public keys, compressed.
/// Domain separator for node signing-key rotations. Spelled out rather than imported, so this
/// signer stays an independent statement of the spec.
pub const NODE_KEY_ROTATION_PREFIX: &[u8] = b"MOLPHA_NODE_KEY_ROTATION_V1";

/// `keccak256(prefix ‖ program_id ‖ node ‖ new_key_compressed)`: the digest both keys sign to
/// rotate `node`'s signing key to `new_key_compressed`.
pub fn key_rotation_digest(
    program_id: &Pubkey,
    node: &Pubkey,
    new_key_compressed: &[u8; 33],
) -> [u8; 32] {
    hashv(&[
        NODE_KEY_ROTATION_PREFIX,
        program_id.as_ref(),
        node.as_ref(),
        new_key_compressed.as_ref(),
    ])
    .to_bytes()
}

/// The proof that rotates `node` from `old` to `new`: `old` signs the digest in the aggregate
/// format, and `new` proves possession over the same digest, which takes the place of the
/// `node_id` in a registration proof.
pub fn key_rotation_proof(
    program_id: &Pubkey,
    node: &Pubkey,
    old: &NodeKey,
    new: &NodeKey,
) -> KeyRotationProof {
    key_rotation_proof_with(program_id, node, old, new, &new.secret)
}

/// [`key_rotation_proof`] with the proof of possession signed by `pop_secret` — for the cases
/// where the signer does not hold the new key's secret.
pub fn key_rotation_proof_with(
    program_id: &Pubkey,
    node: &Pubkey,
    old: &NodeKey,
    new: &NodeKey,
    pop_secret: &[u8; 32],
) -> KeyRotationProof {
    let digest = key_rotation_digest(program_id, node, &new.compressed);
    let old_signature = sign_aggregate_raw(&old.compressed, &old.secret, &digest, None);
    let nonce = hashv(&[b"molpha-test-rotation-pop-nonce".as_ref(), digest.as_ref()]).to_bytes();
    let (pop_sig_r, pop_sig_s) =
        sign_pop_with(program_id, &digest, &new.compressed, pop_secret, &nonce);
    KeyRotationProof {
        new_public_key_compressed: new.compressed,
        old_key_sig_s: old_signature.agg_sig_s,
        old_key_commitment: old_signature.commitment,
        pop_sig_r,
        pop_sig_s,
    }
}

pub fn coalition_pubkey(keys: &[&NodeKey]) -> [u8; 33] {
    let points: Vec<PublicKey> = keys.iter().map(|k| k.public_key()).collect();
    PublicKey::combine(&points)
        .expect("non-degenerate coalition")
        .serialize_compressed()
}

/// Sum of the coalition's secret keys, mod n.
pub fn coalition_secret(keys: &[&NodeKey]) -> [u8; 32] {
    keys.iter()
        .fold([0u8; 32], |acc, k| add_mod_n(&acc, &k.secret))
}

/// Sign `message_hash` for the coalition. `nonce` pins `k` (leave `None` for a derived one).
pub fn sign_aggregate(
    keys: &[&NodeKey],
    message_hash: &[u8; 32],
    nonce: Option<[u8; 32]>,
) -> AggregateSignature {
    let coalition_compressed = coalition_pubkey(keys);
    let coalition_sk = coalition_secret(keys);
    sign_aggregate_raw(&coalition_compressed, &coalition_sk, message_hash, nonce)
}

/// [`sign_aggregate`] over an explicit coalition key pair.
pub fn sign_aggregate_raw(
    coalition_compressed: &[u8; 33],
    coalition_secret: &[u8; 32],
    message_hash: &[u8; 32],
    nonce: Option<[u8; 32]>,
) -> AggregateSignature {
    let k = nonce.unwrap_or_else(|| {
        // Derived, not random: a failing test must reproduce on re-run.
        hashv(&[
            b"molpha-test-nonce-v1",
            coalition_compressed.as_ref(),
            message_hash.as_ref(),
        ])
        .to_bytes()
    });
    let k = reduce_mod_n(&k);
    assert_ne!(k, [0u8; 32], "schnorr nonce must be non-zero mod n");

    let r_point = NodeKey::from_secret(k);
    let mut uncompressed_xy = [0u8; 64];
    uncompressed_xy[..32].copy_from_slice(&r_point.x);
    uncompressed_xy[32..].copy_from_slice(&r_point.y);
    let commitment = eth_address_from_uncompressed_pubkey(uncompressed_xy);

    let px: [u8; 32] = coalition_compressed[1..33].try_into().unwrap();
    let parity = [coalition_compressed[0] & 1];
    let challenge = reduce_mod_n(
        &hashv(&[
            px.as_ref(),
            parity.as_ref(),
            message_hash.as_ref(),
            commitment.as_ref(),
        ])
        .to_bytes(),
    );
    let s = add_mod_n(&k, &mul_mod_n(&challenge, coalition_secret));

    AggregateSignature {
        agg_sig_s: s,
        commitment,
        nonce: k,
    }
}

// ---------------------------------------------------------------------------
// nonce commitment batches
// ---------------------------------------------------------------------------

pub const DOMAIN_COMMITMENT_BATCH: &[u8] = b"MOLPHA_COMMITMENT_BATCH_V1";

/// One slot's committed nonce pair, with the secrets kept so transcripts can be forged.
#[derive(Clone, Debug)]
pub struct CommittedSlot {
    pub slot: u64,
    pub r1_secret: [u8; 32],
    pub r2_secret: [u8; 32],
    pub r1: NodeKey,
    pub r2: NodeKey,
}

impl CommittedSlot {
    pub fn new(slot: u64, r1_secret: [u8; 32], r2_secret: [u8; 32]) -> Self {
        Self {
            slot,
            r1_secret,
            r2_secret,
            r1: NodeKey::from_secret(r1_secret),
            r2: NodeKey::from_secret(r2_secret),
        }
    }

    /// Deterministic slot commitments for a test.
    pub fn derive(domain: &str, slot: u64) -> Self {
        let secret_for = |tag: &str| {
            let mut attempt = 0u32;
            loop {
                let seed = hashv(&[
                    domain.as_bytes(),
                    tag.as_bytes(),
                    &slot.to_be_bytes(),
                    &attempt.to_be_bytes(),
                ])
                .to_bytes();
                if SecretKey::parse(&seed).is_ok() {
                    return seed;
                }
                attempt += 1;
            }
        };
        Self::new(slot, secret_for("r1"), secret_for("r2"))
    }

    pub fn leaf(&self) -> [u8; 32] {
        commitment_leaf(self.slot, &self.r1.x, &self.r1.y, &self.r2.x, &self.r2.y)
    }
}

/// Merkle leaf over raw coordinates, so a test can commit a point that is not on the curve.
pub fn commitment_leaf(
    slot: u64,
    r1_x: &[u8; 32],
    r1_y: &[u8; 32],
    r2_x: &[u8; 32],
    r2_y: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        &[0x00],
        slot.to_be_bytes().as_slice(),
        r1_x.as_slice(),
        r1_y.as_slice(),
        r2_x.as_slice(),
        r2_y.as_slice(),
    ])
    .to_bytes()
}

fn merkle_parent(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    hashv(&[&[0x01], lo.as_slice(), hi.as_slice()]).to_bytes()
}

/// A node's signed batch of per-slot nonce commitments.
///
/// Sorted-sibling merkle tree; odd nodes are promoted, never duplicated.
#[derive(Clone, Debug)]
pub struct CommitmentBatchTree {
    pub slots: Vec<CommittedSlot>,
    levels: Vec<Vec<[u8; 32]>>,
}

impl CommitmentBatchTree {
    pub fn new(slots: Vec<CommittedSlot>) -> Self {
        assert!(!slots.is_empty(), "batch needs at least one slot");
        let mut levels = vec![slots.iter().map(|s| s.leaf()).collect::<Vec<_>>()];
        while levels.last().expect("level").len() > 1 {
            let current = levels.last().expect("level");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                next.push(if pair.len() == 2 {
                    merkle_parent(&pair[0], &pair[1])
                } else {
                    pair[0]
                });
            }
            levels.push(next);
        }
        Self { slots, levels }
    }

    /// A batch of `count` deterministic slots numbered from `first_slot`.
    pub fn derive(domain: &str, first_slot: u64, count: u64) -> Self {
        Self::new(
            (0..count)
                .map(|i| CommittedSlot::derive(domain, first_slot + i))
                .collect(),
        )
    }

    pub fn root(&self) -> [u8; 32] {
        self.levels.last().expect("root level")[0]
    }

    pub fn depth(&self) -> usize {
        self.levels.len() - 1
    }

    pub fn proof(&self, index: usize) -> Vec<[u8; 32]> {
        let mut proof = Vec::new();
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            let sibling = idx ^ 1;
            if sibling < level.len() {
                proof.push(level[sibling]);
            }
            idx /= 2;
        }
        proof
    }
}

/// The message a node signs to commit a batch root.
pub fn commitment_batch_message(
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

/// Sign a batch root with the node's own key (a coalition of one).
#[allow(clippy::too_many_arguments)]
pub fn sign_commitment_batch(
    node: &NodeKey,
    program_id: &Pubkey,
    registry_version: u32,
    node_index: u32,
    epoch: u64,
    batch_meta: &[u8; 32],
    batch_root: &[u8; 32],
) -> AggregateSignature {
    let message = commitment_batch_message(
        program_id,
        registry_version,
        node_index,
        epoch,
        batch_meta,
        batch_root,
    );
    sign_aggregate_raw(&node.compressed, &node.secret, &message, None)
}

/// An honest partial signature over a committed nonce pair: `s = r1 + b*r2 + c*x`.
///
/// Two of these over one slot with different `(b, c)` pairs is the punishable offence.
pub fn forge_partial_transcript(
    slot: &CommittedSlot,
    node: &NodeKey,
    b: &[u8; 32],
    c: &[u8; 32],
) -> [u8; 32] {
    add_mod_n(
        &slot.r1_secret,
        &add_mod_n(&mul_mod_n(b, &slot.r2_secret), &mul_mod_n(c, &node.secret)),
    )
}

// ---------------------------------------------------------------------------
// bitmaps
// ---------------------------------------------------------------------------

/// 256-bit big-endian bitmap with `bits` set — the on-chain `signers_bitmap` encoding.
pub fn bitmap_from_bits(bits: &[usize]) -> [u8; 32] {
    let mut bitmap = molpha_verifier::Bitmap::new();
    for bit in bits {
        bitmap.set_bit(*bit);
    }
    bitmap.to_bytes()
}

pub fn bitmap_bits(bitmap: &[u8; 32]) -> Vec<usize> {
    let bitmap = molpha_verifier::Bitmap::load(bitmap);
    (0..256).filter(|bit| bitmap.bit_set(*bit)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_keys_are_deterministic_and_distinct() {
        let a = node_keys("crypto-unit", 4);
        let b = node_keys("crypto-unit", 4);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.compressed, y.compressed);
        }
        for i in 0..a.len() {
            for j in (i + 1)..a.len() {
                assert_ne!(a[i].compressed, a[j].compressed);
            }
        }
    }

    #[test]
    fn coalition_secret_matches_coalition_pubkey() {
        let keys = node_keys("crypto-unit-coalition", 5);
        let refs: Vec<&NodeKey> = keys.iter().collect();
        let summed_secret = coalition_secret(&refs);
        assert_eq!(
            NodeKey::from_secret(summed_secret).compressed,
            coalition_pubkey(&refs),
            "sum of secrets must generate the sum of public keys"
        );
    }

    #[test]
    fn aggregate_signature_verifies_against_the_verifier() {
        let keys = node_keys("crypto-unit-sign", 3);
        let refs: Vec<&NodeKey> = keys.iter().collect();
        let message_hash = [0x5au8; 32];
        let sig = sign_aggregate(&refs, &message_hash, None);

        let signers: Vec<molpha_verifier::SignerXy> = keys.iter().map(|k| (k.x, k.y)).collect();
        assert!(molpha_verifier::verify_aggregate_over_hash(
            &sig.agg_sig_s,
            &sig.commitment,
            &message_hash,
            &signers,
        )
        .unwrap());
    }

    #[test]
    fn aggregate_signature_does_not_verify_over_another_message() {
        let keys = node_keys("crypto-unit-sign", 3);
        let refs: Vec<&NodeKey> = keys.iter().collect();
        let sig = sign_aggregate(&refs, &[0x5au8; 32], None);

        let signers: Vec<molpha_verifier::SignerXy> = keys.iter().map(|k| (k.x, k.y)).collect();
        assert!(!molpha_verifier::verify_aggregate_over_hash(
            &sig.agg_sig_s,
            &sig.commitment,
            &[0x5bu8; 32],
            &signers,
        )
        .unwrap());
    }

    #[test]
    fn pop_verifies_for_its_own_key() {
        let program_id = molpha::ID;
        let key = &node_keys("crypto-unit-pop", 1)[0];
        let node_id = [0x77u8; 32];
        let (r, s) = sign_pop(&program_id, &node_id, key, &[0x21u8; 32]);
        assert!(molpha_verifier::validate_key_and_verify_pop(
            &program_id.to_bytes(),
            &node_id,
            &key.compressed,
            &r,
            &s,
        )
        .is_ok());
    }

    #[test]
    fn bitmap_round_trips_through_bit_positions() {
        let bits = [0usize, 3, 7, 8, 255];
        assert_eq!(bitmap_bits(&bitmap_from_bits(&bits)), bits.to_vec());
    }
}
