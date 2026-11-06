//! Checks shared by every instruction that registers or rotates a signing key, for nodes and
//! providers alike.

use anchor_lang::prelude::*;
use molpha_verifier::{KeyRotationError, NodePopError};

use crate::error::MolphaError;

/// Whether `key` is a valid ed25519 public key (a point on the curve): a provider's libp2p
/// identity.
pub fn require_valid_p2p_key(key: &[u8; 32]) -> Result<()> {
    require!(
        *key != [0u8; 32] && Pubkey::new_from_array(*key).is_on_curve(),
        MolphaError::InvalidP2pKey
    );
    Ok(())
}

pub fn pop_error(error: NodePopError) -> Error {
    match error {
        NodePopError::InvalidPublicKey => error!(MolphaError::PubkeyNotOnCurve),
        NodePopError::InvalidProof => error!(MolphaError::InvalidPoP),
    }
}

pub fn rotation_error(error: KeyRotationError) -> Error {
    match error {
        KeyRotationError::InvalidPublicKey => error!(MolphaError::PubkeyNotOnCurve),
        KeyRotationError::InvalidProof => error!(MolphaError::InvalidPoP),
        KeyRotationError::InvalidAuthorization => error!(MolphaError::InvalidKeyRotationSignature),
        KeyRotationError::KeyUnchanged => error!(MolphaError::KeyRotationKeyUnchanged),
    }
}
