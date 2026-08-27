//! Ed25519 signature introspection via the instructions sysvar.
//!
//! Clients must place a self-contained Ed25519 verify instruction immediately before settlement.
//! Matching attestation data proves the expected `(pubkey, message)` was verified by the runtime.

use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::error::MolphaError;

/// Ed25519 native program id.
const ED25519_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111");

const SIGNATURE_OFFSETS_START: usize = 2;
const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
const PUBKEY_SERIALIZED_SIZE: usize = 32;
const SELF_CONTAINED_IX_INDEX: u16 = u16::MAX;

#[inline]
fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let bytes: [u8; 2] = data
        .get(offset..offset + 2)
        .and_then(|s| s.try_into().ok())
        .ok_or(error!(MolphaError::InvalidInstructionsSysvar))?;
    Ok(u16::from_le_bytes(bytes))
}

/// Returns `true` if a self-contained Ed25519 instruction attests `(expected_pubkey, expected_message)`.
fn ed25519_instruction_attests(
    data: &[u8],
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
) -> Result<bool> {
    if data.len() < SIGNATURE_OFFSETS_START {
        return Ok(false);
    }
    let num_signatures = data[0] as usize;

    for i in 0..num_signatures {
        let base = SIGNATURE_OFFSETS_START + i * SIGNATURE_OFFSETS_SERIALIZED_SIZE;
        if data.len() < base + SIGNATURE_OFFSETS_SERIALIZED_SIZE {
            return Ok(false);
        }

        // Ed25519 wire layout: signature_offset, signature_ix_index, pubkey_offset, ...
        let pubkey_offset = read_u16_le(data, base + 4)? as usize;
        let pubkey_ix_index = read_u16_le(data, base + 6)?;
        let message_offset = read_u16_le(data, base + 8)? as usize;
        let message_size = read_u16_le(data, base + 10)? as usize;
        let message_ix_index = read_u16_le(data, base + 12)?;

        if pubkey_ix_index != SELF_CONTAINED_IX_INDEX || message_ix_index != SELF_CONTAINED_IX_INDEX
        {
            continue;
        }

        let Some(pubkey) = data.get(pubkey_offset..pubkey_offset + PUBKEY_SERIALIZED_SIZE) else {
            continue;
        };
        if pubkey != expected_pubkey.as_slice() {
            continue;
        }

        let Some(message) = data.get(message_offset..message_offset + message_size) else {
            continue;
        };
        if message == expected_message {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Verify that the instruction immediately preceding the current one is an Ed25519
/// native-program instruction proving that `expected_pubkey` signed `expected_message`.
pub fn verify_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
) -> Result<bool> {
    let current_index = load_current_index_checked(instructions_sysvar)
        .map_err(|_| error!(MolphaError::InvalidInstructionsSysvar))?;
    if current_index == 0 {
        return Ok(false);
    }

    let ed25519_index = current_index - 1;
    let ix = load_instruction_at_checked(ed25519_index as usize, instructions_sysvar)
        .map_err(|_| error!(MolphaError::InvalidInstructionsSysvar))?;
    if ix.program_id != ED25519_PROGRAM_ID {
        return Ok(false);
    }

    ed25519_instruction_attests(&ix.data, expected_pubkey, expected_message)
}
