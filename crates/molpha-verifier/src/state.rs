use anchor_lang::prelude::*;

pub const VIRTUAL_INDEX: u32 = u32::MAX;

#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub authority: Pubkey,
    pub current_node_count: u32,
    pub grace_secs: i64,
    pub redundancy_buffer: u8,
    pub current_version: u32,
    pub previous_version: u32,
    pub previous_expires_at: i64,
    pub previous_node_count: u32,
    pub last_transition_type: RegistryTransitionType,
    pub removed_old_index: u32,
    pub removed_node: Pubkey,
    pub moved_old_index: u32,
    pub moved_node: Pubkey,
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init_genesis(
        &mut self,
        authority: Pubkey,
        redundancy_buffer: u8,
        grace_secs: i64,
        bump: u8,
    ) {
        self.authority = authority;
        self.redundancy_buffer = redundancy_buffer;
        self.current_version = 0;
        self.previous_version = 0;
        self.previous_expires_at = 0;
        self.current_node_count = 0;
        self.previous_node_count = 0;
        self.last_transition_type = RegistryTransitionType::None;
        self.clear_remove_transition_metadata();
        self.grace_secs = grace_secs;
        self.bump = bump;
    }

    pub fn record_add_transition(&mut self, now: i64) {
        let current_version = self.current_version;
        self.previous_version = current_version;
        self.previous_node_count = self.current_node_count;
        self.previous_expires_at = now.saturating_add(self.grace_secs);
        self.current_version = current_version.saturating_add(1);
        self.current_node_count = self.current_node_count.saturating_add(1);
        self.last_transition_type = RegistryTransitionType::Add;
        self.clear_remove_transition_metadata();
    }

    pub fn is_accepted_registry_version(&self, version: u32, now: i64) -> bool {
        version == self.current_version
            || (version == self.previous_version && now <= self.previous_expires_at)
    }

    pub fn is_remove_transition(&self) -> bool {
        matches!(
            self.last_transition_type,
            RegistryTransitionType::RemoveTail | RegistryTransitionType::RemoveSwap
        )
    }

    pub fn clear_remove_transition_metadata(&mut self) {
        self.removed_old_index = 0;
        self.removed_node = Pubkey::default();
        self.moved_old_index = VIRTUAL_INDEX;
        self.moved_node = Pubkey::default();
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum RegistryTransitionType {
    None,
    Add,
    RemoveTail,
    RemoveSwap,
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
