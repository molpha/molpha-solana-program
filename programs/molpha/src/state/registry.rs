use anchor_lang::prelude::*;

use crate::error::MolphaError;

/// Maximum registry membership. Signer bitmaps are fixed 256-bit values.
pub const MAX_REGISTRY_NODES: usize = 256;

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

    pub fn copy_from_previous(&mut self, previous: &Registry, bump: u8) {
        self.version = previous.version + 1;
        self.node_count = previous.node_count;
        self.redundancy_buffer = previous.redundancy_buffer;
        self.bump = bump;
        self.nodes.copy_from_slice(&previous.nodes);
    }

    pub fn append(&mut self, previous: &Registry, node: Pubkey, bump: u8) -> Result<()> {
        require!(
            usize::from(self.node_count) < MAX_REGISTRY_NODES,
            MolphaError::NodeCapExceeded
        );
        require!(
            !previous.is_node_in_registry(node),
            MolphaError::NodeAlreadyInRegistry
        );

        self.copy_from_previous(previous, bump);
        self.nodes[usize::from(self.node_count)] = node.to_bytes();
        self.node_count = self.node_count.checked_add(1).unwrap();
        Ok(())
    }

    pub fn update_buffer(&mut self, previous: &Registry, redundancy_buffer: u8, bump: u8) {
        self.copy_from_previous(previous, bump);
        self.redundancy_buffer = redundancy_buffer;
    }

    pub fn swap_remove(&mut self, previous: &Registry, node: Pubkey, bump: u8) -> Result<()> {
        self.copy_from_previous(previous, bump);
        self.swap_remove_index(self.get_index(node)?)?;
        Ok(())
    }

    pub fn swap_remove_batch(
        &mut self,
        previous: &Registry,
        remove_nodes: &[Pubkey],
        bump: u8,
    ) -> Result<()> {
        self.copy_from_previous(previous, bump);
        for remove_node in remove_nodes {
            if self.is_node_in_registry(*remove_node) {
                self.swap_remove_index(self.get_index(*remove_node)?)?;
            }
        }
        Ok(())
    }

    pub fn get_index(&self, node: Pubkey) -> Result<usize> {
        (0..usize::from(self.node_count))
            .find(|&i| Pubkey::new_from_array(self.nodes[i]) == node)
            .ok_or(error!(MolphaError::InvalidNodeAccount))
    }

    pub fn is_node_in_registry(&self, node: Pubkey) -> bool {
        (0..usize::from(self.node_count)).any(|i| Pubkey::new_from_array(self.nodes[i]) == node)
    }

    fn swap_remove_index(&mut self, index: usize) -> Result<()> {
        let last_index = usize::from(self.node_count - 1);
        self.nodes[index] = self.nodes[last_index];
        self.nodes[last_index] = [0u8; 32];
        self.node_count -= 1;
        Ok(())
    }
}

const _: () = assert!(Registry::SPACE == 8_208);
