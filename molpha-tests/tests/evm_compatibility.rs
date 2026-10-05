//! Cross-chain golden vector.
//!
//! `fixtures/verify-answer-evm.json` carries the shared Molpha attestation fixture: the node key
//! material, duty and coalition the EVM validator is also tested against. Two things have to
//! agree across chains for a Molpha attestation to be portable — the deterministic selection
//! derivation and the attestation message hash — and both are pinned here.
//!
//! The fixture's recorded `messageHash` is the digest the EVM validator signs and verifies for
//! this duty; [`FIXTURE_MESSAGE_HASH`] pins the same value on the Solana side, and the test
//! below checks the two agree. The key material, duty and coalition are live inputs.

use molpha::state::{Feed, Node};
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use molpha_verifier::RegistryView;
use serde_json::Value;
use solana_signer::Signer;

const REDUNDANCY_BUFFER: u8 = 2;

/// `compute_message_hash` (millisecond timestamp) for the fixture duty and its derived
/// coalition.
///
/// The EVM validator must produce this same digest for the same inputs; a change on either side
/// is a cross-chain break, not a local refactor.
const FIXTURE_MESSAGE_HASH: [u8; 32] = [
    0x58, 0x48, 0x9d, 0x3d, 0xed, 0x5f, 0x12, 0x9c, 0x6f, 0xca, 0x09, 0xa1, 0x74, 0xff, 0x0d, 0xcf,
    0x35, 0xc5, 0x8f, 0x70, 0xc5, 0x60, 0xe2, 0xd5, 0xce, 0x53, 0x8e, 0xf1, 0x0c, 0x3b, 0xbb, 0x3e,
];

struct Fixture {
    registry_version: u32,
    signatures_required: u8,
    timestamp: u64,
    source_id: [u8; 32],
    value: [u8; 32],
    /// `messageHash` as recorded in the fixture file.
    recorded_message_hash: [u8; 32],
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
        timestamp: attestation["timestamp"].as_u64().unwrap(),
        source_id: hex32(attestation["sourceId"].as_str().unwrap()),
        value: hex32(attestation["value"].as_str().unwrap()),
        recorded_message_hash: hex32(json["messageHash"].as_str().unwrap()),
        signers_bitmap,
        keys,
    }
}

impl Fixture {
    fn duty(&self) -> Duty {
        Duty::new(self.registry_version, self.signatures_required)
            .source_id(self.source_id)
            .at(self.timestamp)
            .value(self.value)
    }

    fn signer_bits(&self) -> Vec<usize> {
        crypto::bitmap_bits(&self.signers_bitmap)
    }
}

#[test]
fn the_fixture_coalition_is_exactly_the_selection_the_program_derives() {
    let fixture = load();
    static NODES: [[u8; 32]; 256] = [[0u8; 32]; 256];
    let registry = RegistryView {
        version: fixture.registry_version,
        node_count: fixture.keys.len() as u16,
        redundancy_buffer: REDUNDANCY_BUFFER,
        nodes: &NODES,
    };
    let derived = molpha_verifier::derive_selection_bitmap(
        &fixture.source_id,
        fixture.timestamp,
        fixture.signatures_required,
        &registry,
    )
    .expect("selection derives");

    assert_eq!(
        derived.to_bytes(),
        fixture.signers_bitmap,
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
        fixture.duty().message_hash(fixture.signers_bitmap),
        FIXTURE_MESSAGE_HASH,
        "the attestation message encoding changed; the EVM validator must be updated in step"
    );
    assert_eq!(
        fixture.recorded_message_hash, FIXTURE_MESSAGE_HASH,
        "the fixture's recorded messageHash is not the pinned digest"
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
    assert_eq!(feed.value, fixture.value);
    assert_eq!(feed.timestamp, fixture.timestamp);
    assert_eq!(feed.signers_bitmap, fixture.signers_bitmap);
}
