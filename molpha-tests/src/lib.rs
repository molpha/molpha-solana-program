//! Test harness for the Molpha Solana program.
//!
//! Each test builds a fresh [`env::TestEnv`] (its own LiteSVM instance, keypairs and mint), so
//! tests share code but never state and can run in any order or in parallel.

// LiteSVM's `TransactionResult` carries a full `FailedTransactionMetadata` (logs included) in its
// `Err` variant. That size is the point — it is what makes a failing test readable.
#![allow(clippy::result_large_err)]

pub mod crypto;
pub mod ed25519;
pub mod env;
pub mod pda;
pub mod scenario;
pub mod token;
