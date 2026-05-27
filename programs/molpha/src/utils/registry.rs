use anchor_lang::prelude::*;
use libsecp256k1::{PublicKey, PublicKeyFormat};
use solana_keccak_hasher::hashv;

use crate::{error::MolphaError, utils::schnorr::secp256k1_scalar_is_valid_nonzero};

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";
pub const JOB_ID_PREFIX: &[u8] = b"MOLPHA_JOB_V1";
pub const LIBP2P_PEER_ID_PREFIX: &[u8] = b"MOLPHA_LIBP2P_PEER_ID_V1";

pub fn derive_node_id(
    owner: &Pubkey,
    secp256k1_pubkey: &[u8; 33],
    registration_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        NODE_ID_PREFIX,
        owner.as_ref(),
        secp256k1_pubkey.as_ref(),
        registration_salt.as_ref(),
    ])
    .to_bytes()
}

pub fn derive_job_id(owner: &Pubkey, api_config_hash: &[u8; 32]) -> [u8; 32] {
    hashv(&[JOB_ID_PREFIX, owner.as_ref(), api_config_hash.as_ref()]).to_bytes()
}

pub fn hash_libp2p_peer_id(peer_id: &[u8]) -> [u8; 32] {
    hashv(&[LIBP2P_PEER_ID_PREFIX, peer_id]).to_bytes()
}

pub fn decompress_secp256k1_pubkey(compressed: &[u8; 33]) -> Result<([u8; 32], [u8; 32])> {
    let pk = PublicKey::parse_slice(compressed, Some(PublicKeyFormat::Compressed))
        .map_err(|_| error!(MolphaError::PubkeyNotOnCurve))?;
    let uncompressed = pk.serialize();
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(&uncompressed[1..33]);
    y.copy_from_slice(&uncompressed[33..65]);
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_evm_pubkey_decompresses() {
        let compressed: [u8; 33] = [
            0x03, 0xc0, 0x95, 0x27, 0xe9, 0x78, 0xf6, 0xea, 0x69, 0xf0, 0xc6, 0xb7, 0xac, 0x0f,
            0xb6, 0x3a, 0xd0, 0x81, 0xa8, 0xa2, 0x91, 0x15, 0x1c, 0x5a, 0x0b, 0x11, 0x5c, 0xce,
            0x43, 0x57, 0x51, 0xbe, 0x7d,
        ];
        decompress_secp256k1_pubkey(&compressed).expect("fixture pubkey must decompress");
    }
}

/// Stubbed PoP: validates signature scalar shape only (no cryptographic verification yet).
pub fn verify_pop_stub(pop_sig_s: &[u8; 32]) -> Result<()> {
    require!(
        secp256k1_scalar_is_valid_nonzero(pop_sig_s),
        MolphaError::InvalidPoP
    );
    Ok(())
}
