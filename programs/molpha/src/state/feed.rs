use super::{Answer, FeedType};
use anchor_lang::prelude::*;

pub const MAX_HISTORY: usize = 20; // Reduced from 100 to 20

#[account]
#[derive(Default, InitSpace)]
pub struct Feed {
    // Existing feed data
    pub name_hash: [u8; 32],
    pub authority: Pubkey,
    pub feed_type: FeedType,
    pub job_id: [u8; 32],
    pub data_source: Pubkey,
    pub min_signatures_threshold: u8,
    pub frequency: u64,
    #[max_len(60)]
    pub ipfs_cid: String,
    pub latest_answer: Answer,
    #[max_len(MAX_HISTORY)]
    pub answer_history: Vec<Answer>,
    pub history_idx: u64,

    // Integrated subscription data (like SubscriptionRegistry)
    pub subscription_due_time: i64,
    pub price_per_second_scaled: u64,
    pub created_at: i64,
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"feed";
    pub const SPACE: usize = 8 + Feed::INIT_SPACE;

    pub fn is_subscription_active(&self, current_time: i64) -> bool {
        self.subscription_due_time > current_time
    }

    pub fn remaining_subscription_time(&self, current_time: i64) -> i64 {
        if self.subscription_due_time > current_time {
            self.subscription_due_time - current_time
        } else {
            0
        }
    }
}
