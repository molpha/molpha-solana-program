use anchor_lang::prelude::*;
use crate::state::{Answer, Feed};

/// Pyth-compatible price data representation
/// This adapter allows consumers to read Molpha feeds using Pyth-like interfaces
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct PythPrice {
    /// Price value
    pub price: i64,
    /// Confidence interval around the price
    pub conf: u64,
    /// Price exponent (price * 10^expo gives the actual price)
    pub expo: i32,
    /// Publish time as unix timestamp
    pub publish_time: i64,
}

/// Price status enum (Pyth-compatible)
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceStatus {
    Unknown,
    Trading,
    Halted,
    Auction,
}

/// Trait for converting Molpha data to Pyth-compatible format
/// Implement this trait on your Answer struct when the feed represents price data
pub trait ToPythPrice {
    /// Convert to Pyth-compatible price structure
    /// Returns None if the data cannot be interpreted as a price
    fn to_pyth_price(&self, expo: i32) -> Option<PythPrice>;
    
    /// Convert with confidence interval
    fn to_pyth_price_with_conf(&self, expo: i32, conf: u64) -> Option<PythPrice>;
}

impl ToPythPrice for Answer {
    fn to_pyth_price(&self, expo: i32) -> Option<PythPrice> {
        self.to_pyth_price_with_conf(expo, 0)
    }
    
    fn to_pyth_price_with_conf(&self, expo: i32, conf: u64) -> Option<PythPrice> {
        // Interpret the first 8 bytes as i64 price
        // This assumes price feeds store the price as little-endian i64 in the first 8 bytes
        let price_bytes: [u8; 8] = self.value[0..8].try_into().ok()?;
        let price = i64::from_le_bytes(price_bytes);
        
        Some(PythPrice {
            price,
            conf,
            expo,
            publish_time: self.timestamp,
        })
    }
}

/// Trait for feeds that can provide Pyth-compatible price data
pub trait PythCompatible {
    /// Get the current price in Pyth format
    fn get_pyth_price(&self) -> Result<PythPrice>;
    
    /// Get price with staleness check
    fn get_pyth_price_no_older_than(&self, max_age: i64) -> Result<PythPrice>;
    
    /// Get the price exponent for this feed
    fn get_price_exponent(&self) -> i32;
    
    /// Get the price status
    fn get_price_status(&self) -> PriceStatus;
}

impl PythCompatible for Feed {
    fn get_pyth_price(&self) -> Result<PythPrice> {
        let expo = self.get_price_exponent();
        
        // Extract confidence from bytes 8-16 if available
        let conf_bytes: [u8; 8] = self.latest_answer.value[8..16]
            .try_into()
            .unwrap_or([0u8; 8]);
        let conf = u64::from_le_bytes(conf_bytes);
        
        self.latest_answer
            .to_pyth_price_with_conf(expo, conf)
            .ok_or_else(|| error!(crate::error::FeedError::InvalidFeedConfig))
    }
    
    fn get_pyth_price_no_older_than(&self, max_age: i64) -> Result<PythPrice> {
        let now = Clock::get()?.unix_timestamp;
        let age = now - self.latest_answer.timestamp;
        
        require!(
            age <= max_age,
            crate::error::FeedError::InvalidFeedConfig // You may want a specific error
        );
        
        self.get_pyth_price()
    }
    
    fn get_price_exponent(&self) -> i32 {
        // Extract exponent from bytes 16-20 if available
        // Default to -8 (same as most Pyth USD feeds)
        let expo_bytes: [u8; 4] = self.latest_answer.value[16..20]
            .try_into()
            .unwrap_or([248u8, 255, 255, 255]); // -8 as i32 little-endian
        
        i32::from_le_bytes(expo_bytes)
    }
    
    fn get_price_status(&self) -> PriceStatus {
        // Check subscription status and data freshness
        let now = Clock::get().map(|c| c.unix_timestamp).unwrap_or(0);
        
        if !self.is_subscription_active(now) {
            return PriceStatus::Halted;
        }
        
        // If data is fresh (within 2x frequency), consider it trading
        let expected_freshness = (self.frequency as i64) * 2;
        let age = now - self.latest_answer.timestamp;
        
        if age <= expected_freshness {
            PriceStatus::Trading
        } else {
            PriceStatus::Unknown
        }
    }
}

/// Helper functions for encoding price data into Answer.value format
pub mod encoding {
    /// Encode price data into the Answer value format
    /// Layout: [price: i64][conf: u64][expo: i32][reserved: 12 bytes]
    pub fn encode_price_data(price: i64, conf: u64, expo: i32) -> [u8; 32] {
        let mut value = [0u8; 32];
        value[0..8].copy_from_slice(&price.to_le_bytes());
        value[8..16].copy_from_slice(&conf.to_le_bytes());
        value[16..20].copy_from_slice(&expo.to_le_bytes());
        // Remaining 12 bytes reserved for future use
        value
    }
    
    /// Decode price data from Answer value format
    pub fn decode_price_data(value: &[u8; 32]) -> (i64, u64, i32) {
        let price = i64::from_le_bytes(value[0..8].try_into().unwrap());
        let conf = u64::from_le_bytes(value[8..16].try_into().unwrap());
        let expo = i32::from_le_bytes(value[16..20].try_into().unwrap());
        (price, conf, expo)
    }
}

#[cfg(test)]
mod tests {
    use super::encoding;
    
    #[test]
    fn test_price_encoding() {
        let price = 50_000_00000000i64; // $50,000 with -8 exponent
        let conf = 10_00000000u64;       // $10 confidence
        let expo = -8i32;
        
        let encoded = encoding::encode_price_data(price, conf, expo);
        let (decoded_price, decoded_conf, decoded_expo) = encoding::decode_price_data(&encoded);
        
        assert_eq!(price, decoded_price);
        assert_eq!(conf, decoded_conf);
        assert_eq!(expo, decoded_expo);
    }
}

