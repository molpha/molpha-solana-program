// use anchor_lang::prelude::*;
// use anchor_lang::system_program::{self, CreateAccount, Transfer};

// use crate::state::Registry;

// /// Largest account-data increase one instruction may apply.
// ///
// /// The runtime rejects a larger growth, whether it comes from a System Program `CreateAccount`
// /// CPI or from `AccountInfo::resize`, measured from the account's size at the start of that
// /// top-level instruction. `Registry` is bigger than this, so creation is several calls.
// const ALLOC_CHUNK: usize = 10_240;

// /// Grow the versioned registry PDA by one chunk, up to [`Registry::SPACE`].
// ///
// /// Idempotent once the account is full. The payer funds rent for the size reached by this call.
// /// Later instructions take the account with `zero` and `load_init`; they do not create it.
// pub fn allocate_registry(ctx: Context<AllocateRegistry>, version: u32) -> Result<()> {
//     let registry = &ctx.accounts.registry;
//     let target = Registry::SPACE;
//     let current = registry.data_len();
//     if current >= target {
//         return Ok(());
//     }

//     if current == 0 {
//         let space = ALLOC_CHUNK.min(target);
//         let lamports = Rent::get()?.minimum_balance(space);
//         let bump = [ctx.bumps.registry];
//         let version_bytes = version.to_le_bytes();
//         system_program::create_account(
//             CpiContext::new_with_signer(
//                 ctx.accounts.system_program.key(),
//                 CreateAccount {
//                     from: ctx.accounts.payer.to_account_info(),
//                     to: registry.to_account_info(),
//                 },
//                 &[&[Registry::SEED_PREFIX, version_bytes.as_ref(), bump.as_ref()]],
//             ),
//             lamports,
//             space as u64,
//             ctx.program_id,
//         )?;
//         return Ok(());
//     }

//     require_keys_eq!(
//         *registry.owner,
//         *ctx.program_id,
//         crate::error::MolphaError::InvalidRegistryVersion
//     );

//     let next = current.saturating_add(ALLOC_CHUNK).min(target);
//     let needed = Rent::get()?.minimum_balance(next);
//     let have = registry.lamports();
//     if needed > have {
//         system_program::transfer(
//             CpiContext::new(
//                 ctx.accounts.system_program.key(),
//                 Transfer {
//                     from: ctx.accounts.payer.to_account_info(),
//                     to: registry.to_account_info(),
//                 },
//             ),
//             needed - have,
//         )?;
//     }
//     registry.to_account_info().resize(next)?;
//     Ok(())
// }

// #[derive(Accounts)]
// #[instruction(version: u32)]
// pub struct AllocateRegistry<'info> {
//     #[account(mut)]
//     pub payer: Signer<'info>,

//     /// CHECK: version PDA, created or extended here. Seeds bind the address; data stays zero
//     /// until a later instruction initializes the snapshot.
//     #[account(
//         mut,
//         seeds = [Registry::SEED_PREFIX, version.to_le_bytes().as_ref()],
//         bump,
//     )]
//     pub registry: UncheckedAccount<'info>,

//     pub system_program: Program<'info, System>,
// }
