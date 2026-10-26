//! Allocation of registry snapshot accounts.
//!
//! A snapshot is sized to its membership, and an account can grow by at most 10 KiB within one
//! top-level instruction. A snapshot of up to 106 members is therefore created by the
//! instruction that publishes it. A larger one is allocated beforehand by `extend_registry`, in
//! as many instructions as it needs, and the publishing instruction brings it to its final size.

use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Allocate, Assign, CreateAccount, Transfer};

use crate::{error::MolphaError, state::Registry};

/// Bytes an account may grow by within one top-level instruction, CPIs included.
pub const MAX_GROWTH_PER_INSTRUCTION: usize = 10_240;

const _: () = assert!(
    MAX_GROWTH_PER_INSTRUCTION
        == anchor_lang::solana_program::entrypoint::MAX_PERMITTED_DATA_INCREASE
);

/// The accounts that pay for and create a registry version's account.
pub struct RegistryAllocation<'a, 'info> {
    /// The version's PDA: not yet created, or allocated by `extend_registry` and never written.
    pub account: &'a AccountInfo<'info>,
    pub payer: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
    /// The version the PDA is seeded with, and its bump.
    pub version: u32,
    pub bump: u8,
}

impl RegistryAllocation<'_, '_> {
    /// Current length of the account, which is zero while it does not exist.
    ///
    /// Fails if the account has already been written as a registry: a version is published once.
    pub fn allocated_len(&self) -> Result<usize> {
        if *self.account.owner == system_program::ID {
            return Ok(0);
        }
        require_keys_eq!(
            *self.account.owner,
            crate::ID,
            ErrorCode::AccountOwnedByWrongProgram
        );
        let data = self.account.try_borrow_data()?;
        require!(
            data.iter().take(8).all(|byte| *byte == 0),
            ErrorCode::AccountDiscriminatorAlreadySet
        );
        Ok(data.len())
    }

    /// Bring the account to exactly `space` bytes, creating it if it does not exist, with
    /// `payer` funding rent for that size.
    ///
    /// Growing by more than one instruction allows fails with `RegistryNotAllocated`: the
    /// account has to be extended first.
    pub fn resize_to(&self, space: usize) -> Result<()> {
        let current = self.allocated_len()?;
        require!(
            space.saturating_sub(current) <= MAX_GROWTH_PER_INSTRUCTION,
            MolphaError::RegistryNotAllocated
        );

        if *self.account.owner == system_program::ID {
            self.create(space)?;
        } else {
            self.account.resize(space)?;
        }
        self.fund_rent(space)
    }

    /// Grow the account toward `target` bytes by as much as one instruction allows. Returns its
    /// new length. A no-op once it is at least that large.
    pub fn extend_toward(&self, target: usize) -> Result<usize> {
        let current = self.allocated_len()?;
        if current >= target {
            return Ok(current);
        }
        let space = target.min(current + MAX_GROWTH_PER_INSTRUCTION);
        self.resize_to(space)?;
        Ok(space)
    }

    /// Shrink the account to `space` bytes and return to `payer` the rent it no longer needs.
    pub fn settle(&self, space: usize) -> Result<()> {
        self.account.resize(space)?;
        let surplus = self
            .account
            .lamports()
            .saturating_sub(Rent::get()?.minimum_balance(space));
        if surplus > 0 {
            self.account.sub_lamports(surplus)?;
            self.payer.add_lamports(surplus)?;
        }
        Ok(())
    }

    fn create(&self, space: usize) -> Result<()> {
        let version = self.version.to_le_bytes();
        let seeds: &[&[u8]] = &[Registry::SEED_PREFIX, version.as_ref(), &[self.bump]];
        let signer = &[seeds];
        let program = self.system_program.key();

        if self.account.lamports() == 0 {
            return system_program::create_account(
                CpiContext::new_with_signer(
                    program,
                    CreateAccount {
                        from: self.payer.clone(),
                        to: self.account.clone(),
                    },
                    signer,
                ),
                Rent::get()?.minimum_balance(space),
                space as u64,
                &crate::ID,
            );
        }
        // Anyone can send lamports to the address ahead of time, and `create_account` refuses an
        // account that holds any. Allocate and assign it instead; `fund_rent` tops it up.
        system_program::allocate(
            CpiContext::new_with_signer(
                program,
                Allocate {
                    account_to_allocate: self.account.clone(),
                },
                signer,
            ),
            space as u64,
        )?;
        system_program::assign(
            CpiContext::new_with_signer(
                program,
                Assign {
                    account_to_assign: self.account.clone(),
                },
                signer,
            ),
            &crate::ID,
        )
    }

    fn fund_rent(&self, space: usize) -> Result<()> {
        let missing = Rent::get()?
            .minimum_balance(space)
            .saturating_sub(self.account.lamports());
        if missing == 0 {
            return Ok(());
        }
        system_program::transfer(
            CpiContext::new(
                self.system_program.key(),
                Transfer {
                    from: self.payer.clone(),
                    to: self.account.clone(),
                },
            ),
            missing,
        )
    }
}
