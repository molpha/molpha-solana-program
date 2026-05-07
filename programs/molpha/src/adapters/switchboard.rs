use crate::state::{Answer, Feed};
use anchor_lang::prelude::*;

/// Switchboard-compatible aggregator data representation
/// This adapter allows consumers to read Molpha feeds using Switchboard-like interfaces
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct SwitchboardResult {
    /// Decimal result value
    pub result: f64,
    /// Number of successful oracle responses
    pub num_success: u32,
    /// Number of error responses
    pub num_error: u32,
    /// Minimum value from oracle responses
    pub min_response: f64,
    /// Maximum value from oracle responses
    pub max_response: f64,
    /// Standard deviation of responses
    pub std_dev: f64,
    /// Timestamp of last update
    pub timestamp: i64,
}

/// Trait for converting Molpha data to Switchboard-compatible format
pub trait ToSwitchboardResult {
    /// Convert to Switchboard-compatible result structure
    fn to_switchboard_result(&self, decimals: u8) -> Option<SwitchboardResult>;
}

impl ToSwitchboardResult for Answer {
    fn to_switchboard_result(&self, decimals: u8) -> Option<SwitchboardResult> {
        // Layout: [value: i64][std_dev: i64][min: i64][max: i64]
        let value_bytes: [u8; 8] = self.value[0..8].try_into().ok()?;
        let value = i64::from_le_bytes(value_bytes);

        // Extract std_dev, min, max if available
        let std_dev_bytes: [u8; 8] = self.value[8..16].try_into().unwrap_or([0u8; 8]);
        let std_dev = i64::from_le_bytes(std_dev_bytes);

        let min_bytes: [u8; 8] = self.value[16..24]
            .try_into()
            .unwrap_or_else(|_| value_bytes);
        let min = i64::from_le_bytes(min_bytes);

        let max_bytes: [u8; 8] = self.value[24..32]
            .try_into()
            .unwrap_or_else(|_| value_bytes);
        let max = i64::from_le_bytes(max_bytes);

        // Convert to f64 with decimals
        let divisor = 10f64.powi(decimals as i32);

        Some(SwitchboardResult {
            result: value as f64 / divisor,
            num_success: 1, // Single answer for now
            num_error: 0,
            min_response: min as f64 / divisor,
            max_response: max as f64 / divisor,
            std_dev: std_dev as f64 / divisor,
            timestamp: self.timestamp,
        })
    }
}

/// Trait for feeds that can provide Switchboard-compatible aggregator data
pub trait SwitchboardCompatible {
    /// Get the latest aggregated result
    fn get_switchboard_result(&self) -> Result<SwitchboardResult>;

    /// Get result with staleness check
    fn get_switchboard_result_no_older_than(&self, max_age: i64) -> Result<SwitchboardResult>;

    /// Get the number of decimals for this feed
    fn get_decimals(&self) -> u8;
}

impl SwitchboardCompatible for Feed {
    fn get_switchboard_result(&self) -> Result<SwitchboardResult> {
        let decimals = self.get_decimals();

        self.latest_answer
            .to_switchboard_result(decimals)
            .ok_or_else(|| error!(crate::error::FeedError::InvalidFeedConfig))
    }

    fn get_switchboard_result_no_older_than(&self, max_age: i64) -> Result<SwitchboardResult> {
        let now = Clock::get()?.unix_timestamp;
        let age = now - self.latest_answer.timestamp;

        require!(age <= max_age, crate::error::FeedError::InvalidFeedConfig);

        self.get_switchboard_result()
    }

    fn get_decimals(&self) -> u8 {
        // Could be stored in feed metadata or derived from job_id
        // Default to 8 decimals
        8
    }
}

/// Helper functions for encoding aggregator data into Answer.value format
pub mod encoding {
    /// Encode aggregator data into the Answer value format
    /// Layout: [value: i64][std_dev: i64][min: i64][max: i64]
    pub fn encode_aggregator_data(value: i64, std_dev: i64, min: i64, max: i64) -> [u8; 32] {
        let mut result = [0u8; 32];
        result[0..8].copy_from_slice(&value.to_le_bytes());
        result[8..16].copy_from_slice(&std_dev.to_le_bytes());
        result[16..24].copy_from_slice(&min.to_le_bytes());
        result[24..32].copy_from_slice(&max.to_le_bytes());
        result
    }

    /// Decode aggregator data from Answer value format
    pub fn decode_aggregator_data(value: &[u8; 32]) -> (i64, i64, i64, i64) {
        let val = i64::from_le_bytes(value[0..8].try_into().unwrap());
        let std_dev = i64::from_le_bytes(value[8..16].try_into().unwrap());
        let min = i64::from_le_bytes(value[16..24].try_into().unwrap());
        let max = i64::from_le_bytes(value[24..32].try_into().unwrap());
        (val, std_dev, min, max)
    }
}
