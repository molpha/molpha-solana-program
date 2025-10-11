use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct DataSource {
    pub authority: Pubkey,                // Solana address of the owner
    pub data_source_type: DataSourceType, // Type of data source
    pub metadata_hash: [u8; 32],          // Hash of the metadata
    #[max_len(32)]
    pub name: String,   // Name of the data source
    pub created_at: i64,                  // Creation timestamp
    pub bump: u8,                         // PDA bump
}

impl DataSource {
    pub const SEED_PREFIX: &'static [u8] = b"data_source";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum DataSourceType {
    Public,
    Private,
}

impl Default for DataSourceType {
    fn default() -> Self {
        DataSourceType::Private
    }
}

impl DataSourceType {
    pub fn to_seed(&self) -> &[u8] {
        match self {
            Self::Public => &[0],
            Self::Private => &[1],
        }
    }
}

/// DataSourceInit - matches EIP-712 structure exactly
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct DataSourceInfo {
    pub data_source_type: DataSourceType,
    pub metadata_hash: [u8; 32],
    pub name: String,
}
