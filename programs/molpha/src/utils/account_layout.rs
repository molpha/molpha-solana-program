//! Fixed-layout readers for the remaining program-owned account types.

use anchor_lang::prelude::*;

use crate::{error::MolphaError, state::Node};

pub const ANCHOR_DISCRIMINATOR_LEN: usize = 8;

const NODE_NODE_ID: usize = ANCHOR_DISCRIMINATOR_LEN;
const NODE_PUBKEY_X: usize = NODE_NODE_ID + 32 + 32 + 4 + 1;
const NODE_PUBKEY_Y: usize = NODE_PUBKEY_X + 32;

#[inline(always)]
fn read_bytes32(data: &[u8], offset: usize, err: MolphaError) -> Result<[u8; 32]> {
    data.get(offset..offset + 32)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(error!(err))
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

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::{AnchorSerialize, Discriminator};

    use crate::state::NodeStatus;

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
    fn node_account_offsets_match_anchor_serialize() {
        let account = Node {
            owner: Pubkey::new_unique(),
            current_index: 2,
            status: NodeStatus::Active,
            ip: [10, 0, 0, 1],
            port: 9_000,
            locked_amount: 2,
            pending_rewards: 3,
            claimable_rewards: 0,
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
