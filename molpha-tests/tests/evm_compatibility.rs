//! Cross-chain golden vector.
//!
//! `fixtures/verify-answer-evm.json` carries the shared Molpha attestation fixture: the node key
//! material, duty and coalition the EVM validator is also tested against. Two things have to
//! agree across chains for a Molpha attestation to be portable — the deterministic selection
//! derivation and the attestation message hash — and both are pinned here.
//!
//! Note on the fixture file: its `messageHash` and `schnorrSignature` fields are a historical
//! artifact. They are internally consistent (the recorded signature verifies over the recorded
//! hash) but the recorded hash is not what `compute_message_hash` produces for the recorded
//! payload any more, so the message encoding changed after the fixture was generated. The old
//! TypeScript suite never caught that because it re-signed the fixture at call time and ignored
//! the recorded signature. These tests therefore treat the key material, duty and coalition as
//! live inputs, and pin the current message hash as [`FIXTURE_MESSAGE_HASH`] instead.

use molpha::state::{Feed, Node};
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use serde_json::Value;
use solana_signer::Signer;

const REDUNDANCY_BUFFER: u8 = 2;

/// `compute_message_hash` for the fixture duty and its derived coalition.
///
/// The EVM validator must produce this same digest for the same inputs; a change on either side
/// is a cross-chain break, not a local refactor.
const FIXTURE_MESSAGE_HASH: [u8; 32] = [
    0xa3, 0xe5, 0x8f, 0x5c, 0x15, 0x7c, 0x98, 0xca, 0xd4, 0xfd, 0x84, 0x78, 0xfd, 0x39, 0xb1, 0x8d,
    0xa8, 0x5f, 0x4a, 0xd2, 0xd5, 0x81, 0xdb, 0x06, 0xb7, 0x24, 0xf5, 0x3b, 0x55, 0xaf, 0xc5, 0xb3,
];

struct Fixture {
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: u64,
    source_id: [u8; 32],
    value: [u8; 32],
    /// The coalition recorded in the fixture, as a 256-bit signer bitmap.
    signers_bitmap: [u8; 32],
    keys: Vec<NodeKey>,
}

fn hex_to_bytes(text: &str) -> Vec<u8> {
    let clean = text.strip_prefix("0x").unwrap_or(text);
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn hex32(text: &str) -> [u8; 32] {
    hex_to_bytes(text).try_into().expect("32 bytes")
}

/// Parse a decimal secp256k1 scalar into 32 big-endian bytes.
fn decimal_to_bytes32(text: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for digit in text.bytes() {
        let mut carry = (digit - b'0') as u16;
        for byte in out.iter_mut().rev() {
            let value = (*byte as u16) * 10 + carry;
            *byte = value as u8;
            carry = value >> 8;
        }
        assert_eq!(carry, 0, "scalar does not fit in 32 bytes");
    }
    out
}

/// Load the fixture, checking it against itself on the way in.
fn load() -> Fixture {
    let raw = include_str!("../fixtures/verify-answer-evm.json");
    let json: Value = serde_json::from_str(raw).expect("fixture is valid JSON");
    let attestation = &json["attestation"];

    let keys: Vec<NodeKey> = json["secretKeys"]
        .as_array()
        .expect("secretKeys")
        .iter()
        .map(|v| NodeKey::from_secret(decimal_to_bytes32(v.as_str().expect("decimal scalar"))))
        .collect();
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(
            key.compressed.to_vec(),
            hex_to_bytes(json["nodePubkeys"][i].as_str().expect("pubkey hex")),
            "fixture secretKeys[{i}] does not generate nodePubkeys[{i}]"
        );
    }
    assert_eq!(
        keys.len() as u64,
        json["registeredNodeCount"].as_u64().expect("node count")
    );
    assert_eq!(
        json["registryVersion"], attestation["registryVersion"],
        "fixture disagrees with itself about the registry version"
    );

    let bitmap_value = json["schnorrSignature"]["signersBitmap"]
        .as_u64()
        .expect("bitmap");
    let mut signers_bitmap = [0u8; 32];
    signers_bitmap[24..32].copy_from_slice(&bitmap_value.to_be_bytes());

    // `nodeIndexesOneBased` is the human-readable view of the same coalition.
    let indexes: Vec<usize> = json["nodeIndexesOneBased"]
        .as_array()
        .expect("nodeIndexesOneBased")
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        crypto::bitmap_bits(&signers_bitmap)
            .into_iter()
            .map(|bit| bit + 1)
            .collect::<Vec<_>>(),
        indexes
    );

    Fixture {
        registry_version: attestation["registryVersion"].as_u64().unwrap() as u32,
        signatures_required: attestation["signaturesRequired"].as_u64().unwrap() as u8,
        canonical_timestamp: attestation["canonicalTimestamp"].as_u64().unwrap(),
        source_id: hex32(attestation["sourceId"].as_str().unwrap()),
        value: hex32(attestation["value"].as_str().unwrap()),
        signers_bitmap,
        keys,
    }
}

impl Fixture {
    fn duty(&self) -> Duty {
        Duty::new(self.registry_version, self.signatures_required)
            .source_id(self.source_id)
            .at(self.canonical_timestamp)
            .value(self.value)
    }

    fn signer_bits(&self) -> Vec<usize> {
        crypto::bitmap_bits(&self.signers_bitmap)
    }
}

#[test]
fn the_fixture_coalition_is_exactly_the_selection_the_program_derives() {
    let fixture = load();
    let derived = molpha_verifier::derive_selection_bitmap(
        &fixture.source_id,
        fixture.registry_version,
        fixture.canonical_timestamp,
        fixture.keys.len() as u32,
        fixture.signatures_required,
        REDUNDANCY_BUFFER,
    )
    .expect("selection derives");

    assert_eq!(
        derived, fixture.signers_bitmap,
        "the two chains no longer agree on who a duty selects"
    );
    assert_eq!(
        fixture.signer_bits().len(),
        fixture.signatures_required as usize + REDUNDANCY_BUFFER as usize
    );
}

#[test]
fn the_message_hash_for_the_fixture_duty_is_pinned() {
    let fixture = load();
    assert_eq!(
        fixture.duty().payload(fixture.signers_bitmap),
        FIXTURE_MESSAGE_HASH,
        "the attestation message encoding changed; the EVM validator must be updated in step"
    );
}

#[test]
fn the_program_accepts_an_attestation_from_the_fixture_coalition() {
    let fixture = load();
    let mut p = Protocol::builder()
        .redundancy_buffer(REDUNDANCY_BUFFER)
        .start();
    for key in &fixture.keys {
        p.register_node_with_key(key.clone());
    }
    assert_eq!(p.current_version(), fixture.registry_version);

    // Registry rows carry the fixture's keys in the fixture's own order, so bit `i` of the
    // fixture bitmap really does mean `nodePubkeys[i]`.
    let registry = p.env.registry(fixture.registry_version);
    for (bit, node_pda) in registry.nodes.iter().enumerate() {
        let node: Node = p.env.get(node_pda);
        assert_eq!(node.secp256k1_pubkey_x, fixture.keys[bit].x);
        assert_eq!(node.secp256k1_pubkey_y, fixture.keys[bit].y);
    }

    let attestation = p.sign_duty(fixture.duty());
    assert_eq!(attestation.signers_bitmap, fixture.signers_bitmap);
    assert_eq!(attestation.message_hash, FIXTURE_MESSAGE_HASH);

    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &attestation);
    p.env.send_ok(&[ix], &[&submitter]);

    let feed: Feed = p.env.get(&pda::feed(
        &fixture.source_id,
        fixture.signatures_required,
        &submitter.pubkey(),
    ));
    assert_eq!(feed.value, fixture.value.to_vec());
    assert_eq!(feed.canonical_timestamp, fixture.canonical_timestamp);
    assert_eq!(feed.signers_bitmap, fixture.signers_bitmap);
}
