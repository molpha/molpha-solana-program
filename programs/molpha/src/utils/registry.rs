use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::{error::MolphaError, utils::schnorr::secp256k1_scalar_is_valid_nonzero};

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";
pub const JOB_ID_PREFIX: &[u8] = b"MOLPHA_JOB_V1";
pub const LIBP2P_PEER_ID_PREFIX: &[u8] = b"MOLPHA_LIBP2P_PEER_ID_V1";

pub fn derive_job_id(
    owner: &Pubkey,
    api_config_hash: &[u8; 32],
    signatures_required: u8,
) -> [u8; 32] {
    let signatures_required_bytes = [signatures_required];
    hashv(&[
        JOB_ID_PREFIX,
        owner.as_ref(),
        api_config_hash.as_ref(),
        signatures_required_bytes.as_ref(),
    ])
    .to_bytes()
}

/// Stubbed PoP: validates signature scalar shape only (no cryptographic verification yet).
pub fn verify_pop_stub(pop_sig_s: &[u8; 32]) -> Result<()> {
    require!(
        secp256k1_scalar_is_valid_nonzero(pop_sig_s),
        MolphaError::InvalidPoP
    );
    Ok(())
}
