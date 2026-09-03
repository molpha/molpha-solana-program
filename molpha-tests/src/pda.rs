//! Program-derived addresses, mirroring the `seeds = [..]` constraints in the program.
//!
//! Derived here from the same `SEED_PREFIX` constants the program uses, so a seed rename
//! breaks compilation instead of silently pointing tests at a different account.

use anchor_lang::prelude::Pubkey;
use molpha::state::{
    Delegate, Feed, Gateway, Node, Plan, PlanType, ProtocolConfig, PunishmentCase, Registry,
    RegistryState, RoundReceiptRecord, Secp256k1KeyClaim, Subscription,
};
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

/// Authority of the shared node-deposit vault.
pub fn node_deposits() -> Pubkey {
    pda(&[molpha::state::NODE_DEPOSITS_SEED])
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

pub fn subscription_round(
    subscription_owner: &Pubkey,
    gateway: &Pubkey,
    source_id: &[u8; 32],
    signatures_required: u8,
    timestamp: u64,
) -> Pubkey {
    pda(&[
        RoundReceiptRecord::SEED_PREFIX,
        subscription_owner.as_ref(),
        gateway.as_ref(),
        source_id.as_ref(),
        &signatures_required.to_le_bytes(),
        &timestamp.to_le_bytes(),
    ])
}

pub fn x402_round(
    payer_id: &[u8; 32],
    gateway: &Pubkey,
    source_id: &[u8; 32],
    signatures_required: u8,
    canonical_timestamp: u64,
) -> Pubkey {
    pda(&[
        RoundReceiptRecord::X402_SEED_PREFIX,
        payer_id.as_ref(),
        gateway.as_ref(),
        source_id.as_ref(),
        &signatures_required.to_le_bytes(),
        &canonical_timestamp.to_le_bytes(),
    ])
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
