//! The shared provider-bit vectors (`fixtures/provider_bits.vectors`) through the program.
//!
//! Generated in `molpha-verifier` by `scripts/gen_provider_vectors.py`, which signs independently
//! of this crate. Each registry is planted as a snapshot and each case is submitted with
//! `submit_attestation`, whose outcome must follow the vector's `verify` column. The
//! `provenance` column belongs to the library function `verify_provenance`, which the program
//! does not expose; the verifier crate's own tests cover it.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use std::collections::BTreeMap;

use anchor_lang::prelude::Pubkey;
use anchor_lang::Discriminator;
use molpha::error::MolphaError;
use molpha::state::Registry;
use molpha_tests::crypto;
use molpha_tests::env::assert_molpha_error;
use molpha_tests::pda;
use molpha_tests::scenario::Duty;
use molpha_tests::scenario::{Protocol, SignedAttestation};
use molpha_verifier::{Attestation, AttestationPayload, SchnorrSignature};
use solana_signer::Signer;

const VECTORS: &str = include_str!("fixtures/provider_bits.vectors");

struct Record {
    index: usize,
    account: [u8; 32],
    x: [u8; 32],
    y: [u8; 32],
}

struct Reg {
    version: u32,
    buffer: u8,
    /// Node slots and provider slots, holes included.
    node_count: usize,
    provider_count: usize,
    /// Keyed by signer bit: node slot `i` at `i`, provider slot `p` at `64 + p`.
    records: Vec<Record>,
}

struct Case {
    name: String,
    registry: String,
    bitmap: [u8; 32],
    required: u8,
    s: [u8; 32],
    commitment: [u8; 20],
    verify: String,
}

struct Vectors {
    value: [u8; 32],
    source_id: [u8; 32],
    timestamp: u64,
    registries: BTreeMap<String, Reg>,
    cases: Vec<Case>,
}

fn unhex<const N: usize>(hex: &str) -> [u8; N] {
    assert_eq!(hex.len(), 2 * N, "{hex}");
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn parse() -> Vectors {
    let mut v = Vectors {
        value: [0; 32],
        source_id: [0; 32],
        timestamp: 0,
        registries: BTreeMap::new(),
        cases: Vec::new(),
    };
    for line in VECTORS
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f[0] {
            "payload" => {
                v.value = unhex(f[1]);
                v.source_id = unhex(f[2]);
                v.timestamp = f[3].parse().unwrap();
            }
            "program" | "provider" => {}
            "registry" => {
                v.registries.insert(
                    f[1].to_string(),
                    Reg {
                        version: f[2].parse().unwrap(),
                        buffer: f[3].parse().unwrap(),
                        node_count: f[4].parse().unwrap(),
                        provider_count: f[5].parse().unwrap(),
                        records: Vec::new(),
                    },
                );
            }
            "record" => v.registries.get_mut(f[1]).unwrap().records.push(Record {
                index: f[2].parse().unwrap(),
                account: unhex(f[3]),
                x: unhex(f[4]),
                y: unhex(f[5]),
            }),
            "case" => v.cases.push(Case {
                name: f[1].to_string(),
                registry: f[2].to_string(),
                bitmap: unhex(f[3]),
                required: f[4].parse().unwrap(),
                s: unhex(f[5]),
                commitment: unhex(f[6]),
                verify: f[8].to_string(),
            }),
            other => panic!("unknown record {other}"),
        }
    }
    v
}

/// The `Registry` account bytes for `reg`: header, then the node table and the provider table.
fn registry_account(reg: &Reg, bump: u8) -> Vec<u8> {
    let mut data = vec![0u8; Registry::space(reg.node_count + reg.provider_count)];
    data[..8].copy_from_slice(Registry::DISCRIMINATOR);
    // grace_active_until = 0 (current), active_from = 0.
    data[24..28].copy_from_slice(&reg.version.to_le_bytes());
    data[28] = reg.node_count as u8;
    data[29] = reg.provider_count as u8;
    data[30] = reg.buffer;
    data[31] = bump;
    for record in &reg.records {
        let table_index = if record.index < molpha_verifier::PROVIDER_BIT_BASE {
            record.index
        } else {
            reg.node_count + record.index - molpha_verifier::PROVIDER_BIT_BASE
        };
        let at = 32 + 96 * table_index;
        data[at..at + 32].copy_from_slice(&record.account);
        data[at + 32..at + 64].copy_from_slice(&record.x);
        data[at + 64..at + 96].copy_from_slice(&record.y);
    }
    data
}

#[test]
fn the_program_follows_the_provider_bit_vectors() {
    let vectors = parse();
    let mut p = Protocol::start();
    for reg in vectors.registries.values() {
        let key = pda::registry(reg.version);
        let (_, bump) = Pubkey::find_program_address(
            &[Registry::SEED_PREFIX, &reg.version.to_le_bytes()],
            &molpha::ID,
        );
        p.env
            .plant_program_account(&key, registry_account(reg, bump));
    }
    // The vectors' round is stamped at TIMESTAMP; the clock is just after it.
    p.env.warp_to((vectors.timestamp / 1_000) as i64 + 5);

    assert!(vectors.cases.len() >= 22);
    for case in &vectors.cases {
        let reg = &vectors.registries[&case.registry];
        let attestation = Attestation {
            payload: AttestationPayload {
                value: vectors.value,
                source_id: vectors.source_id,
                registry_version: reg.version,
                timestamp: vectors.timestamp,
                signatures_required: case.required,
            },
            signature: SchnorrSignature {
                agg_sig_s: case.s,
                commitment: case.commitment,
                signers_bitmap: case.bitmap,
            },
        };
        // The coalition key a client derives: the sum of the registry's keys at the set bits,
        // zero when a bit has no record (those cases fail before the key is looked at).
        let bits = crypto::bitmap_bits(&case.bitmap);
        let table: BTreeMap<usize, ([u8; 32], [u8; 32])> =
            reg.records.iter().map(|r| (r.index, (r.x, r.y))).collect();
        let coalition_key = bits
            .iter()
            .map(|bit| table.get(bit).copied())
            .collect::<Option<Vec<_>>>()
            .and_then(|xy| molpha_verifier::coalition_key(&xy).ok())
            .unwrap_or(molpha_tests::scenario::ZERO_COALITION_KEY);
        let signed = SignedAttestation {
            duty: Duty::new(reg.version, case.required),
            signers_bitmap: case.bitmap,
            message_hash: attestation.message_hash(),
            signature: crypto::AggregateSignature {
                agg_sig_s: case.s,
                commitment: case.commitment,
                nonce: [0; 32],
            },
            signer_pdas: Vec::new(),
            coalition_key,
        };
        let mut args = signed.submit_args();
        args.attestation = attestation;

        let submitter = p.env.keypair();
        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
        let result = p.env.send(&[ix], &[&submitter]);
        match case.verify.as_str() {
            "ok" => {
                if case.required < p.config().min_signers {
                    // The verifier accepts it (a provider alone is a quorum of one); the program's
                    // own floor on `signatures_required` refuses it before the verifier runs.
                    assert_molpha_error(
                        &result.expect_err(&case.name),
                        MolphaError::SignaturesBelowMinSigners,
                    );
                } else {
                    result.unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
                }
            }
            "InsufficientSigners" => {
                // The program checks the floor and the node quorum before the verifier runs.
                let failed = result.expect_err(&case.name);
                let expected = if case.required < p.config().min_signers {
                    MolphaError::SignaturesBelowMinSigners
                } else {
                    MolphaError::QuorumBelowThreshold
                };
                assert_molpha_error(&failed, expected);
            }
            "InvalidSignersBitmap" => assert_molpha_error(
                &result.expect_err(&case.name),
                MolphaError::InvalidSignersBitmap,
            ),
            "InvalidAggregateSignature" => assert_molpha_error(
                &result.expect_err(&case.name),
                MolphaError::InvalidAggregateSignature,
            ),
            other => panic!("{}: unknown outcome {other}", case.name),
        }
    }
}

/// The copy in this repository is the verifier's file: regenerate it there, then copy it here.
/// Skipped when the sibling checkout is not next to this one.
#[test]
fn the_vectors_are_the_verifiers_file() {
    let sibling = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../molpha-verifier/tests/fixtures/provider_bits.vectors");
    let Ok(original) = std::fs::read_to_string(&sibling) else {
        eprintln!("molpha-verifier checkout not found; skipping the sync check");
        return;
    };
    assert_eq!(
        original, VECTORS,
        "copy molpha-verifier/tests/fixtures/provider_bits.vectors over molpha-tests/tests/fixtures/"
    );
}
