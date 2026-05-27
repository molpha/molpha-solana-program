use anchor_lang::prelude::*;

#[event]
pub struct ProtocolInitialized {
    pub protocol_config: Pubkey,
    pub registry_state: Pubkey,
    pub treasury: Pubkey,
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub initialized_at: i64,
}

#[event]
pub struct NodeRegistered {
    pub node_id: [u8; 32],
    pub new_index: u32,
    pub owner: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub current_version: u32,
    pub security_deposit: u64,
    pub registered_at: i64,
}

#[event]
pub struct NodeDeactivated {
    pub owner: Pubkey,
    pub removed_index: u32,
    pub moved_node: Pubkey,
    pub moved_old_index_or_virtual: u32,
    pub current_version: u32,
    pub deactivated_at: i64,
}

#[event]
pub struct RegistryVersionActivated {
    pub previous_version: u32,
    pub active_version: u32,
    pub previous_expires_at: i64,
    pub activated_at: i64,
}

#[event]
pub struct NodeSlotFinalized {
    pub node_index: u32,
    pub old_status: u8,
    pub new_status: u8,
    pub finalized_at: i64,
}

#[event]
pub struct PlanAdded {
    pub plan_type: u8,
    pub subscription_price: u64,
    pub max_jobs: u32,
    pub max_signers: u8,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

#[event]
pub struct PlanUpdated {
    pub plan_type: u8,
    pub subscription_price: u64,
    pub max_jobs: u32,
    pub max_signers: u8,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

#[event]
pub struct Subscribed {
    pub owner: Pubkey,
    pub plan_type: u8,
    pub prepaid_usdc: u64,
    pub price_snapshot: u64,
    pub valid_until: i64,
}

#[event]
pub struct SubscriptionExtended {
    pub owner: Pubkey,
    pub plan_type: u8,
    pub prepaid_usdc: u64,
    pub valid_until: i64,
}

#[event]
pub struct SubscriptionVaultWithdrawn {
    pub authority: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
}

#[event]
pub struct JobCreated {
    pub job_id: [u8; 32],
    pub owner: Pubkey,
    pub plan_type: u8,
    pub signatures_required: u8,
    pub created_at: i64,
}

#[event]
pub struct DelegateAdded {
    pub job_id: [u8; 32],
    pub delegate: Pubkey,
    pub delegate_count: u8,
}

#[event]
pub struct DelegateRemoved {
    pub job_id: [u8; 32],
    pub delegate: Pubkey,
    pub delegate_count: u8,
}

#[event]
pub struct FeedUpdated {
    pub job_id: [u8; 32],
    pub registry_version: u32,
    pub canonical_timestamp: i64,
    pub value: [u8; 32],
    pub signers_bitmap: [u8; 32],
    pub last_updated_slot: u64,
}
