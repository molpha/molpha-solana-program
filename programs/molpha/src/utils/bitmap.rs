//! EVM-compatible `uint256` bitmap operations + deterministic selection-group derivation.
//!
//! Moved to the standalone `molpha-verifier` crate so downstream programs can verify a
//! `DataUpdate` directly. Re-exported here to preserve existing `crate::utils::bitmap::*`
//! import paths. Note: `derive_group_bitmap` / `validate_bitmap_upper_bits_clear` now return
//! `molpha_verifier::DataUpdateError`; map it via
//! `crate::error::map_data_update_err` at anchor call sites.

pub use molpha_verifier::bitmap::*;
