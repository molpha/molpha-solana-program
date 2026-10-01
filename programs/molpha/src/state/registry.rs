use anchor_lang::prelude::*;

use crate::error::MolphaError;

/// Maximum registry membership. Signer bitmaps are fixed 256-bit values.
pub const MAX_REGISTRY_NODES: usize = 256;

/// Immutable, version-addressed snapshot of the ordered node set and selection policy.
///
/// Only `nodes[..node_count]` is populated. Node addresses are stored as bytes to keep the
/// zero-copy layout POD-safe. `SPACE` includes the Anchor discriminator.
#[account(zero_copy)]
#[repr(C)]
pub struct Registry {
    pub version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub bump: u8,
    pub nodes: [[u8; 32]; MAX_REGISTRY_NODES],
    pub grace_active_until: i64,
    /// Inclusive creation time. A version cannot verify a round dated before it existed.
    pub active_from: i64,
}

impl Registry {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + core::mem::size_of::<Self>();

    pub fn init_genesis(&mut self, redundancy_buffer: u8, active_from: i64, bump: u8) {
        self.set_redundancy_buffer(redundancy_buffer).unwrap();
        self.active_from = active_from;
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

    pub fn copy_from_previous(
        &mut self,
        previous: &mut Registry,
        grace_active_until: i64,
        active_from: i64,
        bump: u8,
    ) {
        self.version = previous.version + 1;
        self.node_count = previous.node_count;
        self.redundancy_buffer = previous.redundancy_buffer;
        self.bump = bump;
        self.nodes.copy_from_slice(&previous.nodes);
        self.active_from = active_from;
        previous.grace_active_until = grace_active_until;
    }

    /// Append a node to a snapshot that was already copied from the previous registry.
    pub fn append_node(&mut self, node: Pubkey) -> Result<()> {
        require!(
            usize::from(self.node_count) < MAX_REGISTRY_NODES,
            MolphaError::NodeCapExceeded
        );
        require!(
            !self.is_node_in_registry(node),
            MolphaError::NodeAlreadyInRegistry
        );

        self.nodes[usize::from(self.node_count)] = node.to_bytes();
        self.node_count = self.node_count.checked_add(1).unwrap();
        Ok(())
    }

    /// Update the redundancy buffer on a snapshot that was already copied from the previous registry.
    pub fn set_redundancy_buffer(&mut self, redundancy_buffer: u8) -> Result<()> {
        require!(
            redundancy_buffer > 0 && redundancy_buffer <= 8,
            MolphaError::InvalidRedundancyBuffer
        );
        require!(
            redundancy_buffer != self.redundancy_buffer,
            MolphaError::RegistryPolicyUnchanged
        );
        self.redundancy_buffer = redundancy_buffer;
        Ok(())
    }

    /// Remove a node from a snapshot that was already copied from the previous registry.
    pub fn swap_remove_node(&mut self, node: Pubkey) -> Result<()> {
        self.swap_remove_index(self.get_index(node)?)?;
        Ok(())
    }

    /// Remove multiple nodes from a snapshot that was already copied from the previous registry.
    pub fn swap_remove_nodes(&mut self, remove_nodes: &[Pubkey]) -> Result<()> {
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

    pub fn is_active(&self, at: i64) -> bool {
        self.active_from <= at && (self.grace_active_until == 0 || self.grace_active_until > at)
    }

    fn swap_remove_index(&mut self, index: usize) -> Result<()> {
        let last_index = usize::from(self.node_count - 1);
        self.nodes[index] = self.nodes[last_index];
        self.nodes[last_index] = [0u8; 32];
        self.node_count -= 1;
        Ok(())
    }
}

const _: () = assert!(Registry::SPACE == 8_224);
