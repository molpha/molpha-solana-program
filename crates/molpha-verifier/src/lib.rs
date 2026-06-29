//! Molpha on-chain registry state and DataUpdate aggregate-Schnorr verification.
//!
//! Downstream Solana programs can verify a Molpha [`DataUpdate`] either from already-resolved
//! signer pubkeys or directly from Molpha `RegistryIndex` accounts.
//!
//! # Usage
//! ```ignore
//! use molpha_verifier::{verify_data_update, DataUpdate};
//!
//! // `ordered_signers` are the signing nodes' (x, y) pubkeys in ascending signers_bitmap bit order.
//! verify_data_update(&payload, node_count, signatures_required, redundancy_buffer, &ordered_signers)?;
//! ```
//!
//! The registry account types are canonical here and re-exported by the Molpha program.

use anchor_lang::prelude::*;

declare_id!("MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8");

pub mod bitmap;
pub mod coalition;
pub mod error;
pub mod message;
pub mod onchain;
pub mod payload;
pub mod scalar;
pub mod selection;
pub mod state;
pub mod verify;

pub use error::DataUpdateError;
pub use onchain::*;
pub use payload::DataUpdate;
pub use state::*;

// High-level verification API.
pub use verify::{
    reconstruct_coalition_key, reconstruct_coalition_key_compressed, verify_aggregate_over_hash,
    verify_data_update, verify_data_update_compressed, SignerXy,
};

// Primitives commonly composed by on-chain callers.
pub use bitmap::{
    bitmap_is_subset_u256, bitmap_load, derive_group_bitmap, effective_selection_size,
    for_each_set_bit_u256,
};
pub use coalition::CoalitionAccumulator;
pub use message::{compute_message_hash, MESSAGE_PREFIX};
pub use scalar::{
    eth_address_from_uncompressed_pubkey, evm_schnorr_ecdsa_inputs,
    secp256k1_scalar_is_valid_nonzero,
};
pub use selection::{derive_selection_bitmap, SELECTION_SEED_PREFIX};
