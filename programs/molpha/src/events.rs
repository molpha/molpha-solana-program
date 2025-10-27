use crate::state::{Answer, DataSourceType, FeedType};
use anchor_lang::prelude::*;

#[event]
pub struct DataSourceCreated {
    pub id: Pubkey,
    pub owner: Pubkey,
    pub name: String,
    pub name_hash: [u8; 32],
    pub data_source_type: DataSourceType,
    pub metadata_hash: [u8; 32],
    pub created_at: i64,
}

#[event]
pub struct FeedCreated {
    pub key: Pubkey,
    pub name: String,
    pub name_hash: [u8; 32],
    pub authority: Pubkey,
    pub feed_type: FeedType,
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    pub ipfs_cid: String,
    pub data_source: Pubkey,
    pub created_at: i64,
    pub subscription_due_time: i64,
    pub total_cost: u64,
    pub price_per_second_scaled: u64,
}

#[event]
pub struct AnswerPublished {
    pub feed: Pubkey,
    pub answer: Answer,
    pub signatures_count: u8,
    pub published_at: i64,
}

#[event]
pub struct SubscriptionExtended {
    pub feed: Pubkey,
    pub authority: Pubkey,
    pub additional_duration: u64,
    pub new_due_time: i64,
    pub extended_at: i64,
}

#[event]
pub struct NodeAdded {
    pub node: Pubkey,
    pub evm_pubkey: Vec<u8>,
    pub authority: Pubkey,
    pub security_deposit: u64,
    pub url: String,
    pub added_at: i64,
}

#[event]
pub struct ProtocolInitialized {
    pub protocol_config: Pubkey,
    pub authority: Pubkey,
    pub initialized_at: i64,
}

#[event]
pub struct FeedConfigUpdated {
    pub feed: Pubkey,
    pub authority: Pubkey,
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    pub job_id: [u8; 32],
    pub price_per_second_scaled: u64,
    pub subscription_due_time: i64,
    pub ipfs_cid: String,
    pub updated_at: i64,

}

#[event]
pub struct ProtocolConfigUpdated {
    pub protocol_config: Pubkey,
    pub authority: Pubkey,
    pub base_price_per_second_scaled: u64,
    pub frequency_coefficient: u64,
    pub signers_coefficient: u64,
    pub reward_percentage: u64,
    pub node_security_deposit: u64,
    pub updated_at: i64,
}

#[event]
pub struct DataSourceUpdated {
    pub data_source: Pubkey,
    pub authority: Pubkey,
    pub metadata_hash: [u8; 32],
    pub updated_at: i64,
}