//! Program-derived addresses, mirroring the `seeds = [..]` constraints in the program.
//!
//! Derived here from the same `SEED_PREFIX` constants the program uses, so a seed rename
//! breaks compilation instead of silently pointing tests at a different account.

use anchor_lang::prelude::Pubkey;
use molpha::state::{
    Delegate, Feed, Gateway, Node, Plan, PlanType, ProtocolConfig, PunishmentCase, Registry,
    RegistryState, RoundReplayKind, RoundReplayMarker, Secp256k1KeyClaim, Subscription,
};
use molpha_verifier::{Attestation, AttestationPayload, SchnorrSignature};
use solana_keccak_hasher::hashv;

pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const INSTRUCTIONS_SYSVAR_ID: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &molpha::ID).0
}

pub fn protocol_config() -> Pubkey {
    pda(&[ProtocolConfig::SEED_PREFIX])
}

pub fn registry_state() -> Pubkey {
    pda(&[RegistryState::SEED_PREFIX])
}

pub fn registry(version: u32) -> Pubkey {
    pda(&[Registry::SEED_PREFIX, &version.to_le_bytes()])
}

pub fn node(owner: &Pubkey) -> Pubkey {
    pda(&[Node::SEED_PREFIX, owner.as_ref()])
}

pub fn secp256k1_key_claim(compressed: &[u8; 33]) -> Pubkey {
    pda(&[
        Secp256k1KeyClaim::SEED_PREFIX,
        &hashv(&[compressed.as_ref()]).to_bytes(),
    ])
}

pub fn plan(plan_type: PlanType) -> Pubkey {
    pda(&[Plan::SEED_PREFIX, &[plan_type as u8]])
}

pub fn subscription(owner: &Pubkey) -> Pubkey {
    pda(&[Subscription::SEED_PREFIX, owner.as_ref()])
}

pub fn delegate(owner: &Pubkey, delegate: &Pubkey) -> Pubkey {
    pda(&[Delegate::SEED_PREFIX, owner.as_ref(), delegate.as_ref()])
}

pub fn gateway(authority: &Pubkey) -> Pubkey {
    pda(&[Gateway::SEED_PREFIX, authority.as_ref()])
}

pub fn feed(source_id: &[u8; 32], signatures_required: u8, submitter: &Pubkey) -> Pubkey {
    pda(&[
        Feed::SEED_PREFIX,
        source_id.as_ref(),
        &signatures_required.to_le_bytes(),
        submitter.as_ref(),
    ])
}

/// Replay marker for a round settled through `gateway` (a gateway PDA) under `round_id`.
pub fn round_replay(gateway: &Pubkey, round_id: &[u8; 32]) -> Pubkey {
    pda(&[
        RoundReplayMarker::SEED_PREFIX,
        gateway.as_ref(),
        round_id.as_ref(),
    ])
}

/// Subscription-path marker: scoped to the consumer that signed the request (owner or delegate).
pub fn subscription_round(
    consumer_authority: &Pubkey,
    gateway: &Pubkey,
    source_id: &[u8; 32],
    signatures_required: u8,
    canonical_timestamp: u64,
) -> Pubkey {
    let attestation = round_attestation(source_id, signatures_required, canonical_timestamp);
    round_replay(
        gateway,
        &RoundReplayMarker::round_id(
            RoundReplayKind::Subscription,
            &attestation,
            &consumer_authority.to_bytes(),
        ),
    )
}

/// x402-path marker: scoped to the rail-scoped `payer_id`.
pub fn x402_round(
    payer_id: &[u8; 32],
    gateway: &Pubkey,
    source_id: &[u8; 32],
    signatures_required: u8,
    canonical_timestamp: u64,
) -> Pubkey {
    let attestation = round_attestation(source_id, signatures_required, canonical_timestamp);
    round_replay(
        gateway,
        &RoundReplayMarker::round_id(RoundReplayKind::X402, &attestation, payer_id),
    )
}

/// `round_id` reads only `source_id`, `signatures_required`, and `canonical_timestamp`.
fn round_attestation(
    source_id: &[u8; 32],
    signatures_required: u8,
    canonical_timestamp: u64,
) -> Attestation {
    Attestation {
        payload: AttestationPayload {
            value: [0; 32],
            source_id: *source_id,
            registry_version: 0,
            signatures_required,
            canonical_timestamp,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0; 32],
            commitment: [0; 20],
            signers_bitmap: [0; 32],
        },
    }
}

pub fn punishment_case(evidence_hash: &[u8; 32]) -> Pubkey {
    pda(&[PunishmentCase::SEED_PREFIX, evidence_hash.as_ref()])
}

/// Associated token account for `owner` on the classic SPL Token program.
pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}
