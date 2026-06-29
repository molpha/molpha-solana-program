//! secp256k1 scalar arithmetic + EVM Schnorr→ECDSA recovery.
//!
//! Moved to the standalone `molpha-verifier` crate so downstream programs can verify a
//! `DataUpdate` directly. Re-exported here to preserve existing `crate::utils::schnorr::*`
//! import paths.

pub use molpha_verifier::scalar::*;
