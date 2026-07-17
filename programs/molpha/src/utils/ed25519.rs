//! Ed25519 native-program signature introspection.
//!
//! Solana verifies Ed25519 signatures in a dedicated native instruction (the Ed25519 program)
//! that runs *outside* the BPF VM. A BPF program proves a signature was checked by reading the
//! instructions sysvar and confirming that a sibling Ed25519 instruction attests to the
//! expected `(pubkey, message)` pair. Because the runtime rejects the whole transaction if any
//! declared Ed25519 signature is invalid, the mere *presence* of a matching attestation is a
//! proof of a valid signature.
//!
//! This module parses the Ed25519 instruction wire layout
//! (`solana_program::ed25519_instruction`) and resolves pubkey/message bytes, which may live in
//! the Ed25519 instruction itself (`instruction_index == u16::MAX`) or in another instruction.

use anchor_lang::prelude::*;
use solana_instructions_sysvar::load_instruction_at_checked;

use crate::error::MolphaError;

/// Ed25519 native program id.
const ED25519_PROGRAM_ID: Pubkey = Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111");

const SIGNATURE_OFFSETS_START: usize = 2;
const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
const PUBKEY_SERIALIZED_SIZE: usize = 32;

#[inline]
fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let bytes: [u8; 2] = data
        .get(offset..offset + 2)
        .and_then(|s| s.try_into().ok())
        .ok_or(error!(MolphaError::InvalidInstructionsSysvar))?;
    Ok(u16::from_le_bytes(bytes))
}

/// Fetch the data slice for an Ed25519 offset reference.
///
/// `u16::MAX` refers to the Ed25519 instruction's own data (`self_data`); any other value is an
/// index into the transaction's instruction list.
fn resolve_instruction_data(
    instructions_sysvar: &AccountInfo,
    self_data: &[u8],
    instruction_index: u16,
) -> Result<Vec<u8>> {
    if instruction_index == u16::MAX {
        Ok(self_data.to_vec())
    } else {
        let ix = load_instruction_at_checked(instruction_index as usize, instructions_sysvar)
            .map_err(|_| error!(MolphaError::InvalidInstructionsSysvar))?;
        Ok(ix.data)
    }
}

/// Returns `true` if a single Ed25519 instruction attests `(expected_pubkey, expected_message)`.
fn ed25519_instruction_attests(
    instructions_sysvar: &AccountInfo,
    self_data: &[u8],
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
) -> Result<bool> {
    if self_data.len() < SIGNATURE_OFFSETS_START {
        return Ok(false);
    }
    let num_signatures = self_data[0] as usize;

    for i in 0..num_signatures {
        let base = SIGNATURE_OFFSETS_START + i * SIGNATURE_OFFSETS_SERIALIZED_SIZE;
        if self_data.len() < base + SIGNATURE_OFFSETS_SERIALIZED_SIZE {
            return Ok(false);
        }

        // Layout: signature_offset, signature_ix_index, pubkey_offset, pubkey_ix_index,
        //         message_offset, message_size, message_ix_index (each u16 LE).
        let pubkey_offset = read_u16_le(self_data, base + 4)? as usize;
        let pubkey_ix_index = read_u16_le(self_data, base + 6)?;
        let message_offset = read_u16_le(self_data, base + 8)? as usize;
        let message_size = read_u16_le(self_data, base + 10)? as usize;
        let message_ix_index = read_u16_le(self_data, base + 12)?;

        let pubkey_data =
            resolve_instruction_data(instructions_sysvar, self_data, pubkey_ix_index)?;
        let Some(pubkey) = pubkey_data.get(pubkey_offset..pubkey_offset + PUBKEY_SERIALIZED_SIZE)
        else {
            continue;
        };
        if pubkey != expected_pubkey.as_slice() {
            continue;
        }

        let message_data =
            resolve_instruction_data(instructions_sysvar, self_data, message_ix_index)?;
        let Some(message) = message_data.get(message_offset..message_offset + message_size) else {
            continue;
        };
        if message == expected_message {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Verify that the transaction contains an Ed25519 native-program instruction proving that
/// `expected_pubkey` signed `expected_message`.
pub fn verify_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
) -> Result<bool> {
    let num_instructions = {
        let data = instructions_sysvar
            .try_borrow_data()
            .map_err(|_| error!(MolphaError::InvalidInstructionsSysvar))?;
        read_u16_le(&data, 0)?
    };

    for i in 0..num_instructions {
        let ix = load_instruction_at_checked(i as usize, instructions_sysvar)
            .map_err(|_| error!(MolphaError::InvalidInstructionsSysvar))?;
        if ix.program_id != ED25519_PROGRAM_ID {
            continue;
        }
        if ed25519_instruction_attests(
            instructions_sysvar,
            &ix.data,
            expected_pubkey,
            expected_message,
        )? {
            return Ok(true);
        }
    }

    Ok(false)
}
