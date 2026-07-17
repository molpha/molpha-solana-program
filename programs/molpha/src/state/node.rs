use anchor_lang::prelude::*;

use crate::error::MolphaError;

pub use molpha_verifier::state::VIRTUAL_INDEX;

/// Slot-keyed registry entry and the single account touched by the round workflows.
///
/// Keyed by `index` (`[SEED_PREFIX, index_le]`), it carries the node's registry slot, the
/// secp256k1 coalition key used for Schnorr verification, and the reward balances accrued during
/// settlement/finalization. Management/lifecycle data lives on the owner-keyed [`NodeOperator`].
#[account]
#[derive(InitSpace)]
pub struct Node {
    pub index: u32,
    /// Operator wallet — used for claim authorization and deactivate validation.
    pub owner: Pubkey,
    /// Validated affine coordinates. `register_node` is the only write path and accepts only a
    /// canonical compressed secp256k1 point, so verifier hot paths may safely skip re-parsing.
    pub secp256k1_pubkey_x: [u8; 32],
    pub secp256k1_pubkey_y: [u8; 32],
    /// Finalized rewards, withdrawable via `claim_node_rewards`.
    pub claimable_rewards: u64,
    pub bump: u8,
}

impl Node {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_node";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        index: u32,
        owner: Pubkey,
        secp256k1_pubkey_x: [u8; 32],
        secp256k1_pubkey_y: [u8; 32],
        bump: u8,
    ) {
        self.index = index;
        self.owner = owner;
        self.secp256k1_pubkey_x = secp256k1_pubkey_x;
        self.secp256k1_pubkey_y = secp256k1_pubkey_y;
        self.claimable_rewards = 0;
        self.bump = bump;
    }
}

/// Parse a slice of signer accounts (typically `ctx.remaining_accounts`) into the plain
/// `NodeEntry`s consumed by `molpha_verifier`. Each account must be owned by this program
/// and hold a valid [`Node`]; the resulting order matches the input order.
pub fn collect_node_entries(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
) -> Result<Vec<molpha_verifier::NodeEntry>> {
    accounts
        .iter()
        .map(|account| {
            require_keys_eq!(
                *account.owner,
                *program_id,
                MolphaError::MissingSignerAccount
            );
            let data = account.try_borrow_data()?;
            let node = Node::try_deserialize(&mut &data[..])
                .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
            Ok(molpha_verifier::NodeEntry {
                index: node.index,
                x: node.secp256k1_pubkey_x,
                y: node.secp256k1_pubkey_y,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::{AnchorSerialize, Discriminator};

    #[test]
    fn node_offsets_match_anchor_serialize() {
        let account = Node {
            index: 5,
            owner: Pubkey::new_from_array([0xAA; 32]),
            secp256k1_pubkey_x: [0x11; 32],
            secp256k1_pubkey_y: [0x22; 32],
            claimable_rewards: 9,
            bump: 7,
        };
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), Node::INIT_SPACE);

        let mut data = vec![0u8; Node::SPACE];
        data[..8].copy_from_slice(Node::DISCRIMINATOR);
        data[8..].copy_from_slice(&body);

        let decoded = Node::try_deserialize(&mut data.as_slice()).expect("deserialize");
        assert_eq!(decoded.index, account.index);
        assert_eq!(decoded.owner, account.owner);
        assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
        assert_eq!(decoded.secp256k1_pubkey_y, account.secp256k1_pubkey_y);
        assert_eq!(decoded.claimable_rewards, account.claimable_rewards);
    }
}
