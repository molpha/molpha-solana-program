use anchor_lang::prelude::*;

use crate::state::{FeedValueKind, NodeStatus, OffenceKind};

#[event]
pub struct ProtocolConfigUpdated {
    pub reward_per_signature: u64,
    pub max_settlement_delay_seconds: u64,
    pub challenger_bounty_bps: u16,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_slots: u64,
    pub x402_round_base: u64,
    pub min_signers: u8,
    pub epoch_len_seconds: u64,
    pub ticket_grace_seconds: u64,
    pub target_tickets_per_epoch: u32,
    pub min_availability_bps: u16,
    pub min_availability_samples: u16,
    pub protocol_fee_bps: u16,
}

#[event]
pub struct ProtocolInitialized {
    pub protocol_config: Pubkey,
    pub registry_state: Pubkey,
    pub registry: Pubkey,
    pub treasury: Pubkey,
    pub node_deposit_vault: Pubkey,
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub initialized_at: i64,
}

#[event]
pub struct NodeRegistered {
    pub node: Pubkey,
    pub owner: Pubkey,
    pub index: u32,
    pub registry_version: u32,
    pub ip: [u8; 4],
    pub port: u16,
    pub security_deposit: u64,
    pub registered_at: i64,
}

#[event]
pub struct NodeEndpointUpdated {
    pub node: Pubkey,
    pub node_authority: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub updated_at: i64,
}

#[event]
pub struct NodeDeactivated {
    pub node: Pubkey,
    pub node_authority: Pubkey,
    pub new_registry_version: u32,
    pub deactivated_at: i64,
}

#[event]
pub struct NodeDepositWithdrawn {
    pub node: Pubkey,
    pub node_authority: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub withdrawn_at: i64,
}

#[event]
pub struct RegistryUpdated {
    pub new_version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub reason: RegistryVersionReason,
    pub updated_at: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub enum RegistryVersionReason {
    NodeAdded,
    NodeRemoved,
    PolicyChanged,
    NodePunished,
    NodeReinstated,
}

#[event]
pub struct PlanAdded {
    pub plan_type: u8,
    pub subscription_price: u64,
    pub max_signers: u8,
    pub max_delegates: u32,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

#[event]
pub struct PlanUpdated {
    pub plan_type: u8,
    pub subscription_price: u64,
    pub max_signers: u8,
    pub max_delegates: u32,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

#[event]
pub struct Subscribed {
    pub owner: Pubkey,
    pub plan_type: u8,
    pub prepaid_usdc: u64,
    pub price_snapshot: u64,
    pub subscribed_at: i64,
    pub valid_until: i64,
}

#[event]
pub struct SubscriptionExtended {
    pub owner: Pubkey,
    pub plan_type: u8,
    pub extended_at: i64,
    pub valid_until: i64,
}

#[event]
pub struct GatewayRegistered {
    pub authority: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub registered_at: i64,
}

#[event]
pub struct GatewayDeactivated {
    pub gateway: Pubkey,
    pub authority: Pubkey,
    pub deactivated_at: i64,
}

#[event]
pub struct DelegateAdded {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub delegate_count: u32,
    pub added_at: i64,
}

#[event]
pub struct DelegateRemoved {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub delegate_count: u32,
    pub removed_at: i64,
}

#[event]
pub struct FeedUpdated {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub timestamp: u64,
    pub value: [u8; 32],
    pub value_kind: FeedValueKind,
    pub signers_bitmap: [u8; 32],
    pub last_updated_slot: u64,
}

#[event]
pub struct RewardsClaimed {
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub claimed_at: i64,
}

#[event]
pub struct OffenceProven {
    pub offence: OffenceKind,
    pub guilty_bitmap: [u8; 32],
    pub evidence_hash: [u8; 32],
    pub reporter: Pubkey,
    pub slashed_total: u64,
    pub bounty_paid: u64,
    pub proven_at: i64,
}

#[event]
pub struct NodePunished {
    pub node: Pubkey,
    pub authority: Pubkey,
    pub offence: OffenceKind,
    pub slashed_locked: u64,
    pub slashed_rewards: u64,
    pub new_status: NodeStatus,
    pub punished_at: i64,
}

#[event]
pub struct NodeReinstated {
    pub node: Pubkey,
    pub node_authority: Pubkey,
    pub index: u32,
    pub registry_version: u32,
    pub reinstated_at: i64,
}

#[event]
pub struct RevenueSwept {
    pub epoch: u64,
    pub gross: u64,
    pub protocol_fee: u64,
    pub pool_share: u64,
    pub swept_at: i64,
}

#[event]
pub struct TicketSubmitted {
    pub epoch: u64,
    pub ticket_id: [u8; 32],
    /// Gateway PDA whose authority signed the round dispatch.
    pub gateway: Pubkey,
    pub submitter: Pubkey,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
}

/// Selected nodes left uncounted because the epoch roster was full.
#[event]
pub struct TicketRosterOverflow {
    pub epoch: u64,
    pub ticket_id: [u8; 32],
    pub uncounted: u32,
}

#[event]
pub struct TicketMarkerClosed {
    pub ticket_marker: Pubkey,
    pub submitter: Pubkey,
    pub closed_at: i64,
}

#[event]
pub struct EpochCreated {
    pub epoch: u64,
    pub start: i64,
    pub end: i64,
    pub ticket_grace: i64,
    pub tau: u64,
}

#[event]
pub struct EpochFinalized {
    pub epoch: u64,
    pub pool: u64,
    pub tickets: u32,
    pub roster_len: u16,
    pub weight_total: u64,
    pub weighted_count: u16,
    pub finalized_at: i64,
}

#[event]
pub struct EpochRewardCredited {
    pub epoch: u64,
    pub node: Pubkey,
    pub weight: u64,
    pub amount: u64,
    /// The node was tombstoned; its share stays in the epoch residual.
    pub forfeited: bool,
}

#[event]
pub struct EpochClosed {
    pub epoch: u64,
    pub residual: u64,
    pub rolled_into: u64,
    pub closed_at: i64,
}

#[event]
pub struct ProtocolReserveWithdrawn {
    pub destination: Pubkey,
    pub amount: u64,
    pub withdrawn_at: i64,
}
