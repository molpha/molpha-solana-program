use anchor_lang::prelude::*;

use crate::state::{FeedValueKind, NodeStatus, OffenceKind};

#[event]
pub struct ProtocolConfigUpdated {
    pub reward_per_signature: u64,
    pub max_settlement_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_slots: u64,
    pub liveliness_freeze_threshold: u16,
    pub x402_round_base: u64,
    pub min_signers: u8,
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
    pub owner: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub updated_at: i64,
}

#[event]
pub struct NodeDeactivated {
    pub node: Pubkey,
    pub owner: Pubkey,
    pub new_registry_version: u32,
    pub deactivated_at: i64,
}

#[event]
pub struct NodeLivelinessPenalized {
    pub node: Pubkey,
    pub round_record: Pubkey,
    pub score: u16,
}

#[event]
pub struct NodeFrozenForInactivity {
    pub node: Pubkey,
    pub owner: Pubkey,
    pub score: u16,
    pub frozen_until: i64,
    pub new_registry_version: u32,
    pub frozen_at: i64,
}

#[event]
pub struct NodeDepositWithdrawn {
    pub node: Pubkey,
    pub owner: Pubkey,
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
pub struct GatewayRegistered {
    pub authority: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
    pub bond: u64,
    pub registered_at: i64,
}

#[event]
pub struct GatewayDeactivated {
    pub gateway: Pubkey,
    pub authority: Pubkey,
    pub withdrawable_slot: u64,
    pub deactivated_at: i64,
}

#[event]
pub struct GatewayBondWithdrawn {
    pub gateway: Pubkey,
    pub amount: u64,
    pub withdrawn_slot: u64,
    pub withdrawn_at: i64,
}

#[event]
pub struct DelegateAdded {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub max_signers: u8,
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
pub struct DelegateStatusChanged {
    pub delegate: Pubkey,
    pub owner: Pubkey,
    pub old_status: u8,
    pub new_status: u8,
    pub updated_at: i64,
}

#[event]
pub struct FeedUpdated {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub canonical_timestamp: u64,
    pub value: [u8; 32],
    pub value_kind: FeedValueKind,
    pub signers_bitmap: [u8; 32],
    pub last_updated_slot: u64,
}

#[event]
pub struct RoundSettled {
    pub gateway: Pubkey,
    pub source_id: [u8; 32],
    pub canonical_timestamp: u64,
    pub credited_count: u32,
    pub reward_per_signature: u64,
    pub recorded_slot: u64,
    pub recorded_at: i64,
}

#[event]
pub struct RoundFinalized {
    pub round_record: Pubkey,
    pub finalized_slot: u64,
    pub finalized_at: i64,
}

#[event]
pub struct RoundVoided {
    pub round_record: Pubkey,
    pub gateway: Pubkey,
    pub round_restored: bool,
    pub voided_at: i64,
}

#[event]
pub struct GatewaySlashed {
    pub gateway: Pubkey,
    pub slashed_bond: u64,
    pub challenger_bounty: u64,
    pub slashed_at: i64,
}

#[event]
pub struct RewardsClaimed {
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub claimed_at: i64,
}

#[event]
pub struct X402RoundSettled {
    pub payer_id: [u8; 32],
    pub payment_rail: u16,
    pub price: u64,
    pub source_id: [u8; 32],
    pub canonical_timestamp: u64,
    pub gateway: Pubkey,
    pub signers_bitmap: [u8; 32],
    pub credited_count: u32,
    pub recorded_slot: u64,
    pub recorded_at: i64,
}

#[event]
pub struct EquivocationProven {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub evidence_hash: [u8; 32],
    pub reporter: Pubkey,
    pub slashed_total: u64,
    pub bounty_paid: u64,
}

#[event]
pub struct UnsolicitedSigningProven {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
    pub signers_bitmap: [u8; 32],
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub evidence_hash: [u8; 32],
    pub reporter: Pubkey,
    pub slashed_total: u64,
    pub bounty_paid: u64,
}

#[event]
pub struct CommitmentReuseProven {
    pub registry_version: u32,
    pub commitment_addr: [u8; 20],
    pub source_id_a: [u8; 32],
    pub signatures_required_a: u8,
    pub canonical_timestamp_a: u64,
    pub signers_bitmap_a: [u8; 32],
    pub source_id_b: [u8; 32],
    pub signatures_required_b: u8,
    pub canonical_timestamp_b: u64,
    pub signers_bitmap_b: [u8; 32],
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub evidence_hash: [u8; 32],
    pub reporter: Pubkey,
    pub slashed_total: u64,
    pub bounty_paid: u64,
}

#[event]
pub struct NonceReuseProven {
    pub registry_version: u32,
    pub node_index: u32,
    pub epoch: u64,
    pub batch_meta: [u8; 32],
    pub batch_root: [u8; 32],
    pub slot: u64,
    pub r1_x: [u8; 32],
    pub r1_y: [u8; 32],
    pub r2_x: [u8; 32],
    pub r2_y: [u8; 32],
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub evidence_hash: [u8; 32],
    pub reporter: Pubkey,
    pub slashed_total: u64,
    pub bounty_paid: u64,
}

#[event]
pub struct NodePunished {
    pub node: Pubkey,
    pub owner: Pubkey,
    pub offence: OffenceKind,
    pub slashed_locked: u64,
    pub slashed_rewards: u64,
    pub new_status: NodeStatus,
    pub punished_at: i64,
}

#[event]
pub struct NodeReinstated {
    pub node: Pubkey,
    pub owner: Pubkey,
    pub index: u32,
    pub registry_version: u32,
    pub reinstated_at: i64,
}
