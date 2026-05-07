use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};

/// Message payload sent to EVM chains when a new node is registered
/// This keeps the payload minimal - only EVM-relevant data
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct NodeRegistrationMessage {
    /// Compressed secp256k1 public key (33 bytes) for EVM chains
    pub evm_pubkey: [u8; 33],

    /// Security deposit amount locked by the node
    pub security_deposit: u64,

    /// Unix timestamp when the node was registered
    pub registered_at: i64,
}

impl NodeRegistrationMessage {
    pub fn new(evm_pubkey: Vec<u8>, security_deposit: u64, registered_at: i64) -> Result<Self> {
        require!(evm_pubkey.len() == 33, WormholeError::InvalidEVMPubkey);

        let mut pubkey_array = [0u8; 33];
        pubkey_array.copy_from_slice(&evm_pubkey);

        Ok(Self {
            evm_pubkey: pubkey_array,
            security_deposit,
            registered_at,
        })
    }

    /// Serialize the message to bytes for Wormhole transmission
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.try_to_vec()
            .map_err(|_| WormholeError::SerializationError.into())
    }
}

#[error_code]
pub enum WormholeError {
    #[msg("Invalid EVM public key length - must be 33 bytes")]
    InvalidEVMPubkey,

    #[msg("Failed to serialize message")]
    SerializationError,
}
