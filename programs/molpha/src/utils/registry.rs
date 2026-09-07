use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::state::{Registry, RegistryState};

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";
pub const LIBP2P_PEER_ID_PREFIX: &[u8] = b"MOLPHA_LIBP2P_PEER_ID_V1";

pub fn derive_node_id(
    owner: Pubkey,
    secp256k1_pubkey_compressed: &[u8; 33],
    registration_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        NODE_ID_PREFIX,
        owner.as_ref(),
        secp256k1_pubkey_compressed.as_ref(),
        registration_salt.as_ref(),
    ])
    .to_bytes()
}

/// Initialize the next registry account, copy state from the current snapshot, then run `apply`.
pub fn with_registry_snapshot<'info, F, R>(
    registry_state: &mut Account<'info, RegistryState>,
    current: &AccountLoader<'info, Registry>,
    next: &AccountLoader<'info, Registry>,
    grace_active_until: i64,
    bump: u8,
    apply: F,
) -> Result<R>
where
    F: FnOnce(&Registry, &mut Registry) -> Result<R>,
{
    let mut current = current.load_mut()?;
    let mut next = next.load_init()?;
    next.copy_from_previous(&mut current, grace_active_until, bump);
    registry_state.advance_version();
    apply(&current, &mut next)
}
