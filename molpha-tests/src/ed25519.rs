//! Self-contained Ed25519 precompile instruction, as `settle_subscription_round` expects it.
//!
//! `utils::ed25519::verify_ed25519_signature` re-parses this layout out of the instructions
//! sysvar, so the encoding is spelled out here instead of delegating to an SDK helper — a test
//! that builds the instruction with the same code the program parses proves nothing.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;

pub const ED25519_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111");

const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
const SIGNATURE_OFFSETS_START: usize = 2;
const SELF_CONTAINED: u16 = u16::MAX;

/// One `(pubkey, signature, message)` triple verified inside the precompile instruction itself.
pub fn ed25519_instruction(pubkey: &[u8; 32], signature: &[u8; 64], message: &[u8]) -> Instruction {
    let pubkey_offset = SIGNATURE_OFFSETS_START + SIGNATURE_OFFSETS_SERIALIZED_SIZE;
    let signature_offset = pubkey_offset + 32;
    let message_offset = signature_offset + 64;

    let mut data = Vec::with_capacity(message_offset + message.len());
    data.push(1); // num_signatures
    data.push(0); // padding
    data.extend_from_slice(&(signature_offset as u16).to_le_bytes());
    data.extend_from_slice(&SELF_CONTAINED.to_le_bytes());
    data.extend_from_slice(&(pubkey_offset as u16).to_le_bytes());
    data.extend_from_slice(&SELF_CONTAINED.to_le_bytes());
    data.extend_from_slice(&(message_offset as u16).to_le_bytes());
    data.extend_from_slice(&(message.len() as u16).to_le_bytes());
    data.extend_from_slice(&SELF_CONTAINED.to_le_bytes());
    data.extend_from_slice(pubkey);
    data.extend_from_slice(signature);
    data.extend_from_slice(message);

    Instruction {
        program_id: ED25519_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}

/// Sign `message` with `signer` and wrap it in an Ed25519 precompile instruction.
pub fn sign_ed25519(signer: &Keypair, message: &[u8]) -> Instruction {
    let signature: [u8; 64] = signer.sign_message(message).into();
    ed25519_instruction(&signer.pubkey().to_bytes(), &signature, message)
}
