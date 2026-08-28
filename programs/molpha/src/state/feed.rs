use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

/// Whether [`Feed::value`] holds the raw oracle payload or a digest of a longer payload.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum FeedValueKind {
    /// `value` is the submitted payload (`len <= 32`).
    Value,
    /// `value` is `keccak256(payload)` for payloads longer than 32 bytes.
    Hash,
}

#[account]
#[derive(InitSpace)]
pub struct Feed {
    pub source_id: [u8; 32],
    /// Stored oracle bytes: raw payload (`FeedValueKind::Value`) or keccak digest
    /// (`FeedValueKind::Hash`). Never longer than [`Feed::STORED_VALUE_LEN`].
    #[max_len(32)]
    pub value: Vec<u8>,
    pub value_kind: FeedValueKind,
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub signers_bitmap: [u8; 32],
    pub registry_version: u32,
    pub bump: u8,
}

pub struct FeedInitParams {
    pub source_id: [u8; 32],
    pub value: [u8; 32],
    pub value_kind: FeedValueKind,
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub bump: u8,
}

pub struct FeedUpdateParams {
    pub value: [u8; 32],
    pub value_kind: FeedValueKind,
    pub canonical_timestamp: u64,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_feed";
    /// Max bytes stored on the account (raw value or hash digest).
    pub const STORED_VALUE_LEN: usize = 32;
    /// Upper bound for submitted instruction payloads (hashed when longer than 32).
    pub const MAX_VALUE_LEN: usize = 256;
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    /// Compact a submitted payload for on-chain storage.
    ///
    /// - `len <= 32` → store bytes as-is, [`FeedValueKind::Value`]
    /// - `len > 32` → store `keccak256(payload)`, [`FeedValueKind::Hash`]
    pub fn encode_stored_value(raw: &[u8]) -> Result<([u8; 32], FeedValueKind)> {
        require!(
            raw.len() <= Self::MAX_VALUE_LEN,
            crate::error::MolphaError::InvalidValueLength
        );
        if raw.len() <= Self::STORED_VALUE_LEN {
            Ok((raw.try_into().unwrap(), FeedValueKind::Value))
        } else {
            Ok((hashv(&[raw]).to_bytes(), FeedValueKind::Hash))
        }
    }

    pub fn init(&mut self, params: &FeedInitParams) {
        self.source_id = params.source_id;
        self.value = params.value.to_vec();
        self.value_kind = params.value_kind;
        self.canonical_timestamp = params.canonical_timestamp;
        self.signatures_required = params.signatures_required;
        self.registry_version = params.registry_version;
        self.signers_bitmap = params.signers_bitmap;
        self.bump = params.bump;
    }

    pub fn update(&mut self, params: &FeedUpdateParams) {
        self.value = params.value.to_vec();
        self.value_kind = params.value_kind;
        self.canonical_timestamp = params.canonical_timestamp;
        self.registry_version = params.registry_version;
        self.signers_bitmap = params.signers_bitmap;
        self.bump = params.bump;
    }
}
