//! Fixed-layout account field offsets and optional zero-copy readers.
//!
//! Bankrun CU measurement (2026-05): Anchor `try_deserialize` on `RegistryIndex`
//! is cheaper than manual offset reads on BPF for the data-update signer loop.
//! The hot path uses `CoalitionAccumulator` (Jacobian adds + trusted stored `(x, y)`)
//! instead of per-signer `PublicKey::parse` / `PublicKey::combine`.

use anchor_lang::prelude::*;
use libsecp256k1::util::{TAG_PUBKEY_EVEN, TAG_PUBKEY_ODD};
use libsecp256k1::{
    curve::{Affine, Field, Jacobian},
    PublicKey, PublicKeyFormat,
};

use crate::{
    error::MolphaError,
    state::{Node, RegistryIndex, RegistryState, RegistryTransitionType, VIRTUAL_INDEX},
};

pub const ANCHOR_DISCRIMINATOR_LEN: usize = 8;

// RegistryIndex field offsets (after 8-byte Anchor discriminator).
const REGISTRY_INDEX_INDEX: usize = ANCHOR_DISCRIMINATOR_LEN;
const REGISTRY_INDEX_NODE_ID: usize = REGISTRY_INDEX_INDEX + 4;
const REGISTRY_INDEX_PUBKEY_X: usize = REGISTRY_INDEX_NODE_ID + 32;
const REGISTRY_INDEX_PUBKEY_Y: usize = REGISTRY_INDEX_PUBKEY_X + 32;

// Node field offsets (after 8-byte Anchor discriminator).
const NODE_NODE_ID: usize = ANCHOR_DISCRIMINATOR_LEN;
const NODE_PUBKEY_X: usize = NODE_NODE_ID + 32 + 32 + 4 + 1; // node_id + owner + current_index + status
const NODE_PUBKEY_Y: usize = NODE_PUBKEY_X + 32;

#[inline(always)]
fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(error!(MolphaError::InvalidRegistryIndexAccount))?;
    Ok(u32::from_le_bytes(bytes))
}

#[inline(always)]
fn read_bytes32(data: &[u8], offset: usize, err: MolphaError) -> Result<[u8; 32]> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or(error!(err))?;
    Ok(bytes)
}

/// Map a previous-version signer bit to the `RegistryIndex` slot that holds its pubkey.
#[inline(always)]
pub fn expected_registry_index(
    bit_pos: u32,
    registry: &RegistryState,
    apply_remove_remap: bool,
) -> u32 {
    if !apply_remove_remap {
        return bit_pos;
    }
    if bit_pos == registry.removed_old_index {
        VIRTUAL_INDEX
    } else if registry.last_transition_type == RegistryTransitionType::RemoveSwap
        && bit_pos == registry.moved_old_index
    {
        registry.removed_old_index
    } else {
        bit_pos
    }
}

/// Previous-version verification during remove grace requires remove-transition metadata.
#[inline(always)]
pub fn validate_remove_transition_for_previous(registry: &RegistryState) -> Result<()> {
    require!(
        registry.is_remove_transition(),
        MolphaError::InvalidTransitionAccount
    );
    Ok(())
}

/// Read secp256k1 affine coordinates from a `RegistryIndex` in `remaining_accounts`.
#[inline(always)]
pub fn load_signer_registry_index(
    data: &[u8],
    expected_index: u32,
    expected_node_id: Option<&[u8; 32]>,
) -> Result<([u8; 32], [u8; 32])> {
    require!(
        data.len() >= ANCHOR_DISCRIMINATOR_LEN
            && data[..ANCHOR_DISCRIMINATOR_LEN] == *RegistryIndex::DISCRIMINATOR,
        MolphaError::InvalidRegistryIndexAccount
    );
    if let Some(node_id) = expected_node_id {
        let actual = read_bytes32(
            data,
            REGISTRY_INDEX_NODE_ID,
            MolphaError::InvalidRegistryIndexAccount,
        )?;
        require!(actual == *node_id, MolphaError::InvalidRegistryIndexAccount);
    }
    read_registry_index_secp(data, expected_index)
}

/// Read `index` and secp256k1 affine coordinates from a `RegistryIndex` without Borsh.
#[inline(always)]
pub fn read_registry_index_secp(data: &[u8], expected_index: u32) -> Result<([u8; 32], [u8; 32])> {
    require!(
        data.len() >= RegistryIndex::SPACE,
        MolphaError::InvalidRegistryIndexAccount
    );
    let index = read_u32_le(data, REGISTRY_INDEX_INDEX)?;
    require!(
        index == expected_index,
        MolphaError::InvalidRegistryIndexAccount
    );
    let x = read_bytes32(
        data,
        REGISTRY_INDEX_PUBKEY_X,
        MolphaError::InvalidRegistryIndexAccount,
    )?;
    let y = read_bytes32(
        data,
        REGISTRY_INDEX_PUBKEY_Y,
        MolphaError::InvalidRegistryIndexAccount,
    )?;
    Ok((x, y))
}

/// Read `node_id` and secp256k1 affine coordinates from a `Node` without Borsh.
#[inline(always)]
pub fn read_node_secp_xy(data: &[u8], expected_node_id: &[u8; 32]) -> Result<([u8; 32], [u8; 32])> {
    require!(
        data.len() >= Node::SPACE,
        MolphaError::InvalidTransitionAccount
    );
    let node_id = read_bytes32(data, NODE_NODE_ID, MolphaError::InvalidTransitionAccount)?;
    require!(
        node_id == *expected_node_id,
        MolphaError::InvalidTransitionAccount
    );
    let x = read_bytes32(data, NODE_PUBKEY_X, MolphaError::InvalidTransitionAccount)?;
    let y = read_bytes32(data, NODE_PUBKEY_Y, MolphaError::InvalidTransitionAccount)?;
    Ok((x, y))
}

/// Load a curve point from registry-stored `(x, y)` without an on-curve re-check.
///
/// Coordinates are validated at registration; the data-update hot path only needs field parsing.
#[inline(always)]
pub fn affine_from_stored_secp_xy(x: &[u8; 32], y: &[u8; 32]) -> Result<Affine> {
    require!(
        *x != [0u8; 32] && *y != [0u8; 32],
        MolphaError::InvalidAggregateSignature
    );
    let mut fx = Field::default();
    let mut fy = Field::default();
    if !fx.set_b32(x) || !fy.set_b32(y) {
        return Err(error!(MolphaError::InvalidAggregateSignature));
    }
    let mut ge = Affine::default();
    ge.set_xy(&fx, &fy);
    Ok(ge)
}

/// Incremental coalition accumulator (`Σ signer pubkeys`) without `PublicKey::combine` / `Vec`.
#[derive(Default)]
pub struct CoalitionAccumulator {
    jacobian: Jacobian,
    has_point: bool,
}

impl CoalitionAccumulator {
    #[inline(always)]
    pub fn add_stored_xy(&mut self, x: &[u8; 32], y: &[u8; 32]) -> Result<()> {
        let ge = affine_from_stored_secp_xy(x, y)?;
        if !self.has_point {
            self.jacobian = Jacobian::from_ge(&ge);
            self.has_point = true;
        } else {
            self.jacobian = self.jacobian.add_ge(&ge);
        }
        Ok(())
    }

    #[inline(always)]
    pub fn compressed_pubkey(&self) -> Result<[u8; 33]> {
        require!(self.has_point, MolphaError::InvalidAggregateSignature);
        if self.jacobian.is_infinity() {
            return Err(error!(MolphaError::InvalidAggregateSignature));
        }
        let mut elem = Affine::from_gej(&self.jacobian);
        elem.x.normalize_var();
        elem.y.normalize_var();
        let mut out = [0u8; 33];
        let mut x_be = [0u8; 32];
        elem.x.fill_b32(&mut x_be);
        out[1..33].copy_from_slice(&x_be);
        out[0] = if elem.y.is_odd() {
            TAG_PUBKEY_ODD
        } else {
            TAG_PUBKEY_EVEN
        };
        Ok(out)
    }
}

/// Build a `libsecp256k1::PublicKey` from stored affine coordinates (on-curve check, no decompress).
///
/// `uncompressed_scratch` is reused across the signer loop (65-byte `0x04 || x || y`).
#[inline(always)]
pub fn public_key_from_affine_xy(
    uncompressed_scratch: &mut [u8; 65],
    x: &[u8; 32],
    y: &[u8; 32],
) -> Result<PublicKey> {
    require!(
        *x != [0u8; 32] && *y != [0u8; 32],
        MolphaError::InvalidAggregateSignature
    );
    uncompressed_scratch[0] = 0x04;
    uncompressed_scratch[1..33].copy_from_slice(x);
    uncompressed_scratch[33..65].copy_from_slice(y);
    PublicKey::parse_slice(uncompressed_scratch.as_ref(), Some(PublicKeyFormat::Full))
        .map_err(|_| error!(MolphaError::InvalidAggregateSignature))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::prelude::Pubkey;
    use anchor_lang::{AnchorSerialize, Discriminator};

    use crate::state::{NodeStatus, RegistryState, RegistryTransitionType};

    /// On-chain account bytes: 8-byte discriminator + serialized body.
    fn registry_index_bytes(account: &RegistryIndex) -> Vec<u8> {
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), RegistryIndex::INIT_SPACE);
        let mut data = vec![0u8; RegistryIndex::SPACE];
        data[..ANCHOR_DISCRIMINATOR_LEN].copy_from_slice(RegistryIndex::DISCRIMINATOR);
        data[ANCHOR_DISCRIMINATOR_LEN..].copy_from_slice(&body);
        data
    }

    fn node_account_bytes(account: &Node) -> Vec<u8> {
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), Node::INIT_SPACE);
        let mut data = vec![0u8; Node::SPACE];
        data[..ANCHOR_DISCRIMINATOR_LEN].copy_from_slice(Node::DISCRIMINATOR);
        data[ANCHOR_DISCRIMINATOR_LEN..].copy_from_slice(&body);
        data
    }

    #[test]
    fn coalition_accumulator_matches_public_key_combine() {
        let fixture_compressed: [[u8; 33]; 3] = [
            [
                0x03, 0xc0, 0x95, 0x27, 0xe9, 0x78, 0xf6, 0xea, 0x69, 0xf0, 0xc6, 0xb7, 0xac, 0x0f,
                0xb6, 0x3a, 0xd0, 0x81, 0xa8, 0xa2, 0x91, 0x15, 0x1c, 0x5a, 0x0b, 0x11, 0x5c, 0xce,
                0x43, 0x57, 0x51, 0xbe, 0x7d,
            ],
            [
                0x02, 0x64, 0xa7, 0x27, 0x04, 0xf3, 0x9f, 0x8d, 0xd1, 0x7f, 0x20, 0xd7, 0x1c, 0x5b,
                0x21, 0xf3, 0x7b, 0x58, 0x52, 0x65, 0x6b, 0xc0, 0x55, 0x54, 0x42, 0xbf, 0x72, 0x72,
                0x22, 0xf2, 0x9d, 0x7e, 0x58,
            ],
            [
                0x02, 0x75, 0xae, 0x1e, 0x3d, 0xac, 0x00, 0xeb, 0x7d, 0xf0, 0x2e, 0x9f, 0xe8, 0xd9,
                0x70, 0x9c, 0x8a, 0x2c, 0x09, 0xa1, 0x1e, 0xd4, 0xf7, 0xd9, 0xaa, 0x46, 0xa7, 0xde,
                0xa6, 0xcf, 0x37, 0x6d, 0x7f,
            ],
        ];
        let pks: Vec<PublicKey> = fixture_compressed
            .iter()
            .map(|c| {
                PublicKey::parse_slice(c, Some(PublicKeyFormat::Compressed))
                    .expect("fixture compressed key")
            })
            .collect();
        let combined = PublicKey::combine(&pks).expect("combine");
        let mut acc = CoalitionAccumulator::default();
        for pk in &pks {
            let full = pk.serialize();
            let x: [u8; 32] = full[1..33].try_into().unwrap();
            let y: [u8; 32] = full[33..65].try_into().unwrap();
            acc.add_stored_xy(&x, &y).expect("accumulate");
        }
        assert_eq!(
            acc.compressed_pubkey().expect("compressed"),
            combined.serialize_compressed()
        );
    }

    #[test]
    fn registry_index_offsets_match_anchor_serialize() {
        let account = RegistryIndex {
            index: 5,
            node: Pubkey::new_from_array([0xAA; 32]),
            secp256k1_pubkey_x: [0x11; 32],
            secp256k1_pubkey_y: [0x22; 32],
            bump: 7,
        };
        let data = registry_index_bytes(&account);
        assert_eq!(data.len(), RegistryIndex::SPACE);

        let decoded = RegistryIndex::try_deserialize(&mut data.as_slice()).expect("deserialize");
        let (x, y) = read_registry_index_secp(&data, 5).expect("zero-copy read");

        assert_eq!(decoded.index, 5);
        assert_eq!(decoded.secp256k1_pubkey_x, x);
        assert_eq!(decoded.secp256k1_pubkey_y, y);
    }

    fn remove_swap_registry() -> RegistryState {
        RegistryState {
            authority: Pubkey::default(),
            redundancy_buffer: 0,
            current_version: 2,
            previous_version: 1,
            previous_expires_at: 9_999,
            current_node_count: 7,
            previous_node_count: 8,
            last_transition_type: RegistryTransitionType::RemoveSwap,
            removed_old_index: 1,
            removed_node: Pubkey::new_from_array([0xAA; 32]),
            moved_old_index: 3,
            moved_node: Pubkey::new_from_array([0xBB; 32]),
            grace_secs: 0,
            bump: 0,
        }
    }

    #[test]
    fn expected_registry_index_remove_swap_remaps_removed_and_moved() {
        let registry = remove_swap_registry();
        assert_eq!(expected_registry_index(1, &registry, true), VIRTUAL_INDEX);
        assert_eq!(expected_registry_index(3, &registry, true), 1);
        assert_eq!(expected_registry_index(0, &registry, true), 0);
    }

    #[test]
    fn expected_registry_index_remove_tail_remaps_removed_only() {
        let mut registry = remove_swap_registry();
        registry.last_transition_type = RegistryTransitionType::RemoveTail;
        registry.removed_old_index = 2;
        registry.moved_old_index = VIRTUAL_INDEX;
        registry.moved_node = Pubkey::default();
        assert_eq!(expected_registry_index(2, &registry, true), VIRTUAL_INDEX);
        assert_eq!(expected_registry_index(0, &registry, true), 0);
    }

    #[test]
    fn expected_registry_index_without_remove_remap_is_identity() {
        let registry = remove_swap_registry();
        assert_eq!(expected_registry_index(4, &registry, false), 4);
    }

    #[test]
    fn node_account_offsets_match_anchor_serialize() {
        let account = Node {
            owner: Pubkey::new_unique(),
            current_index: 2,
            status: NodeStatus::Active,
            ip: [10, 0, 0, 1],
            port: 9_000,
            locked_amount: 2,
            pending_rewards: 3,
            registered_at: 4,
            deactivated_at: 5,
            withdrawable_at: 6,
            bump: 9,
        };
        let data = node_account_bytes(&account);
        assert_eq!(data.len(), Node::SPACE);

        let decoded = Node::try_deserialize(&mut data.as_slice()).expect("deserialize");
        assert_eq!(decoded.owner, account.owner);
        assert_eq!(decoded.current_index, account.current_index);
        assert_eq!(decoded.port, account.port);
    }
}
