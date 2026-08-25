use anchor_lang::prelude::*;

use crate::error::MolphaError;

/// Maximum registry membership. Signer bitmaps are fixed 256-bit values.
pub const MAX_REGISTRY_NODES: usize = 256;

/// Mutable pointer to the current immutable registry snapshot.
#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub current_version: u32,
    pub bump: u8,
}

impl RegistryState {
    // The singleton retains the legacy one-seed address; version snapshots use the same prefix
    // plus a version seed, so the PDA namespaces remain disjoint.
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, bump: u8) {
        self.current_version = 0;
        self.bump = bump;
    }

    pub fn require_next_version(&self, new_version: u32) -> Result<()> {
        let expected = self
            .current_version
            .checked_add(1)
            .ok_or(error!(MolphaError::RegistryVersionOverflow))?;
        require!(new_version == expected, MolphaError::InvalidRegistryVersion);
        Ok(())
    }
}

/// Immutable, version-addressed snapshot of the ordered node set and selection policy.
///
/// Only `nodes[..node_count]` is populated. Node addresses are stored as bytes to keep the
/// zero-copy layout POD-safe and exactly 8,208 bytes including the Anchor discriminator.
#[account(zero_copy)]
#[repr(C)]
pub struct Registry {
    pub version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub bump: u8,
    pub nodes: [[u8; 32]; MAX_REGISTRY_NODES],
}

impl Registry {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + core::mem::size_of::<Self>();

    pub fn init_genesis(&mut self, redundancy_buffer: u8, bump: u8) {
        self.version = 0;
        self.node_count = 0;
        self.redundancy_buffer = redundancy_buffer;
        self.bump = bump;
        self.nodes = [[0u8; 32]; MAX_REGISTRY_NODES];
    }

    pub fn node(&self, index: usize) -> Result<Pubkey> {
        require!(
            index < usize::from(self.node_count),
            MolphaError::InvalidNodeIndex
        );
        Ok(Pubkey::new_from_array(self.nodes[index]))
    }

    /// Framework-agnostic view for [`molpha_verifier`] resolution / verify helpers.
    pub fn view(&self) -> molpha_verifier::RegistryView<'_> {
        molpha_verifier::RegistryView {
            version: self.version,
            node_count: self.node_count,
            redundancy_buffer: self.redundancy_buffer,
            nodes: &self.nodes,
        }
    }

    pub fn copy_from_previous(&mut self, previous: &Registry, version: u32, bump: u8) {
        self.version = version;
        self.node_count = previous.node_count;
        self.redundancy_buffer = previous.redundancy_buffer;
        self.bump = bump;
        self.nodes.copy_from_slice(&previous.nodes);
    }

    pub fn append(&mut self, node: Pubkey, max_node_count: u32) -> Result<u16> {
        let index = usize::from(self.node_count);
        require!(
            index < MAX_REGISTRY_NODES && (index as u32) < max_node_count,
            MolphaError::NodeCapExceeded
        );
        self.nodes[index] = node.to_bytes();
        self.node_count = self
            .node_count
            .checked_add(1)
            .ok_or(error!(MolphaError::NodeCapExceeded))?;
        Ok(index as u16)
    }

    pub fn swap_remove(&mut self, remove_index: u16, node: Pubkey) -> Result<()> {
        let remove_index = usize::from(remove_index);
        require!(
            remove_index < usize::from(self.node_count),
            MolphaError::InvalidNodeIndex
        );
        require_keys_eq!(
            Pubkey::new_from_array(self.nodes[remove_index]),
            node,
            MolphaError::InvalidNodeAccount
        );

        let last_index = usize::from(self.node_count - 1);
        self.nodes[remove_index] = self.nodes[last_index];
        self.nodes[last_index] = [0u8; 32];
        self.node_count -= 1;
        Ok(())
    }
}

const _: () = assert!(Registry::SPACE == 8_208);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_version_is_strict_and_checked() {
        let state = RegistryState {
            current_version: 41,
            bump: 1,
        };
        assert!(state.require_next_version(42).is_ok());
        assert!(state.require_next_version(41).is_err());
        assert!(state.require_next_version(43).is_err());

        let exhausted = RegistryState {
            current_version: u32::MAX,
            bump: 1,
        };
        assert!(exhausted.require_next_version(0).is_err());
    }

    #[test]
    fn snapshot_copy_and_swap_remove_do_not_mutate_source() {
        let first = Pubkey::new_unique();
        let second = Pubkey::new_unique();
        let third = Pubkey::new_unique();
        let mut source = Registry {
            version: 7,
            node_count: 3,
            redundancy_buffer: 2,
            bump: 1,
            nodes: [[0; 32]; MAX_REGISTRY_NODES],
        };
        source.nodes[0] = first.to_bytes();
        source.nodes[1] = second.to_bytes();
        source.nodes[2] = third.to_bytes();

        let mut next = Registry {
            version: 0,
            node_count: 0,
            redundancy_buffer: 0,
            bump: 0,
            nodes: [[0; 32]; MAX_REGISTRY_NODES],
        };
        next.copy_from_previous(&source, 8, 2);
        next.swap_remove(1, second).unwrap();

        assert_eq!(source.node_count, 3);
        assert_eq!(source.node(1).unwrap(), second);
        assert_eq!(next.version, 8);
        assert_eq!(next.node_count, 2);
        assert_eq!(next.node(1).unwrap(), third);
        assert_eq!(next.nodes[2], [0; 32]);
    }
}
