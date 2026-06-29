use anchor_lang::prelude::*;

use crate::error::MolphaError;

/// Parse a slice of signer accounts (typically `ctx.remaining_accounts`) into the plain
/// `RegistryIndexEntry`s consumed by `molpha_verifier`. Each account must be owned by this program
/// and hold a valid `RegistryIndex`; the resulting order matches the input order.
pub fn collect_registry_index_entries(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
) -> Result<Vec<molpha_verifier::RegistryIndexEntry>> {
    accounts
        .iter()
        .map(|account| {
            require_keys_eq!(
                *account.owner,
                *program_id,
                MolphaError::MissingSignerAccount
            );
            let data = account.try_borrow_data()?;
            let registry_index = RegistryIndex::try_deserialize(&mut &data[..])
                .map_err(|_| error!(MolphaError::InvalidRegistryIndexAccount))?;
            Ok(molpha_verifier::RegistryIndexEntry {
                index: registry_index.index,
                x: registry_index.secp256k1_pubkey_x,
                y: registry_index.secp256k1_pubkey_y,
            })
        })
        .collect()
}

#[account]
#[derive(InitSpace)]
pub struct RegistryIndex {
    pub index: u32,
    pub node: Pubkey,
    pub secp256k1_pubkey_x: [u8; 32],
    pub secp256k1_pubkey_y: [u8; 32],
    pub bump: u8,
}

impl RegistryIndex {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry_index";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        index: u32,
        node: Pubkey,
        secp256k1_pubkey_x: [u8; 32],
        secp256k1_pubkey_y: [u8; 32],
        bump: u8,
    ) {
        self.index = index;
        self.node = node;
        self.secp256k1_pubkey_x = secp256k1_pubkey_x;
        self.secp256k1_pubkey_y = secp256k1_pubkey_y;
        self.bump = bump;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::{AnchorSerialize, Discriminator};

    #[test]
    fn registry_index_offsets_match_anchor_serialize() {
        let account = RegistryIndex {
            index: 5,
            node: Pubkey::new_from_array([0xAA; 32]),
            secp256k1_pubkey_x: [0x11; 32],
            secp256k1_pubkey_y: [0x22; 32],
            bump: 7,
        };
        let mut body = Vec::new();
        account.serialize(&mut body).unwrap();
        assert_eq!(body.len(), RegistryIndex::INIT_SPACE);

        let mut data = vec![0u8; RegistryIndex::SPACE];
        data[..8].copy_from_slice(RegistryIndex::DISCRIMINATOR);
        data[8..].copy_from_slice(&body);

        let decoded = RegistryIndex::try_deserialize(&mut data.as_slice()).expect("deserialize");
        assert_eq!(decoded.index, account.index);
        assert_eq!(decoded.secp256k1_pubkey_x, account.secp256k1_pubkey_x);
        assert_eq!(decoded.secp256k1_pubkey_y, account.secp256k1_pubkey_y);
    }
}
