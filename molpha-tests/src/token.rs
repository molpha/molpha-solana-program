//! Minimal SPL Token / Associated Token instruction builders and account readers.
//!
//! Hand-encoded rather than pulled from `spl-token` so the tests bind to the wire format the
//! deployed program actually sees, and so nothing here can drift with an SDK bump.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};

use crate::pda::{ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_PROGRAM_ID};

pub const MINT_LEN: usize = 82;
pub const TOKEN_ACCOUNT_LEN: usize = 165;

/// `InitializeMint2 { decimals, mint_authority, freeze_authority: None }`.
pub fn initialize_mint2(mint: &Pubkey, mint_authority: &Pubkey, decimals: u8) -> Instruction {
    let mut data = Vec::with_capacity(35);
    data.push(20);
    data.push(decimals);
    data.extend_from_slice(mint_authority.as_ref());
    data.push(0); // freeze_authority: None
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    }
}

/// `MintTo { amount }`.
pub fn mint_to(
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(7);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

/// `Transfer { amount }` — used only to move test funds around, never by the program under test.
pub fn transfer(
    source: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*source, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

/// Associated Token Account `CreateIdempotent`.
pub fn create_ata_idempotent(funder: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*funder, true),
            AccountMeta::new(crate::pda::ata(owner, mint), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(anchor_lang::solana_program::system_program::ID, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: vec![1],
    }
}

/// Decoded fields of an SPL token account that the tests actually assert on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenAccountState {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}

pub fn unpack_token_account(data: &[u8]) -> TokenAccountState {
    assert!(
        data.len() >= TOKEN_ACCOUNT_LEN,
        "not an SPL token account ({} bytes)",
        data.len()
    );
    TokenAccountState {
        mint: Pubkey::try_from(&data[0..32]).unwrap(),
        owner: Pubkey::try_from(&data[32..64]).unwrap(),
        amount: u64::from_le_bytes(data[64..72].try_into().unwrap()),
    }
}

/// Raw SPL mint account data — used to plant a mint owned by a *different* token program,
/// which is the only way to exercise the program's token-program constraints.
pub fn pack_mint(mint_authority: &Pubkey, supply: u64, decimals: u8) -> Vec<u8> {
    let mut data = vec![0u8; MINT_LEN];
    data[0..4].copy_from_slice(&1u32.to_le_bytes()); // mint_authority: Some
    data[4..36].copy_from_slice(mint_authority.as_ref());
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = decimals;
    data[45] = 1; // is_initialized
    data[46..50].copy_from_slice(&0u32.to_le_bytes()); // freeze_authority: None
    data
}
