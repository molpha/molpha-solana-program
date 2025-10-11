use crate::state::{Answer, DataSourceType, FeedType};
use anchor_lang::prelude::*;

#[event]
pub struct DataSourceCreated {
    pub id: Pubkey,
    pub owner: Pubkey,
    pub name: String,
    pub data_source_type: DataSourceType,
    pub metadata_hash: [u8; 32],
    pub created_at: i64,
}

#[event]
pub struct FeedCreated {
    pub key: Pubkey,
    pub authority: Pubkey,
    pub feed_type: FeedType,
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    pub ipfs_cid: String,
    pub data_source: Pubkey,
    pub created_at: i64,
    pub subscription_due_time: i64,
    pub total_cost: u64,
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
    pub new_ipfs_cid: String,
    pub updated_at: i64,
}

#[event]
pub struct FeedToppedUp {
    pub feed: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub new_balance: u64,
    pub topped_up_at: i64,
}


#[event]
pub struct ProtocolConfigUpdated {
    pub protocol_config: Pubkey,
    pub authority: Pubkey,
    pub updated_at: i64,
}
