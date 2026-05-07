use anchor_lang::prelude::*;

#[error_code]
pub enum NodeRegistryError {
    #[msg("The given node public key is a zero key.")]
    ZeroPubkey,
    #[msg("Maximum number of nodes reached.")]
    MaxNodesReached,
    #[msg("Node is already part of the registry.")]
    NodeAlreadyAdded,
    #[msg("Node is not part of the registry.")]
    NodeNotFound,
    #[msg("Not enough valid signatures from registered nodes.")]
    NotEnoughSignatures,
    #[msg("Failed to parse Ed25519 instruction.")]
    InvalidEd25519Instruction,
    #[msg("Insufficient security deposit.")]
    InsufficientSecurityDeposit,
    #[msg("Invalid URL.")]
    InvalidURL,
    #[msg("Invalid EVM secp256k1 public key.")]
    InvalidEvmPubkey,
}

#[error_code]
pub enum FeedError {
    #[msg("Invalid feed configuration.")]
    InvalidFeedConfig,
    #[msg("This operation is not supported for this feed type.")]
    NotSupported,
    #[msg("The caller is not the owner of the feed.")]
    NotFeedOwner,
    #[msg("The provided answer has a timestamp in the past.")]
    PastTimestamp,
    #[msg("The provided answer has a timestamp in the future.")]
    FutureTimestamp,
    #[msg("The answer value cannot be empty.")]
    ZeroValue,
    #[msg("Insufficient balance for this operation.")]
    InsufficientBalance,
    #[msg("Not enough valid signatures from registered nodes.")]
    NotEnoughSignatures,
    #[msg("Invalid or inaccessible data source.")]
    InvalidDataSource,
    #[msg("Subscription has expired.")]
    SubscriptionExpired,
    #[msg("Minimum subscription time is 1 day.")]
    MinimumSubscriptionTime,
    #[msg("Minimum extension time is 1 day.")]
    MinimumExtensionTime,
    #[msg("Insufficient priority fee budget.")]
    InsufficientPriorityFeeBudget,
    #[msg("Too many nodes provided. Maximum is 25 nodes per transaction.")]
    TooManyNodes,
}

#[error_code]
pub enum DataSourceError {
    #[msg("Data source with this ID already exists.")]
    DataSourceAlreadyExists,
    #[msg("Invalid data source data format.")]
    InvalidDataSourceData,
    #[msg("Invalid data source type.")]
    InvalidDataSourceType,
}

#[error_code]
pub enum ProtocolConfigError {
    #[msg("Too many target chains configured. Maximum is 10 chains.")]
    TooManyTargetChains,
}

#[error_code]
pub enum VerifyError {
    #[msg("Signed data is too stale for the requested max staleness.")]
    DataTooStale,
    #[msg("Not enough signers in bitmap for required threshold.")]
    InsufficientSigners,
    #[msg("Invalid aggregate signature.")]
    InvalidSignature,
    #[msg("Signers bitmap contains out-of-range indices.")]
    InvalidSignersBitmap,
    #[msg("Malformed aggregated R point.")]
    MalformedRPoint,
    #[msg("Signer is not in the round's selected group bitmap.")]
    SignerNotInGroup,
    #[msg("Missing or extra Node account for a signer in the bitmap.")]
    MissingSignerAccount,
    #[msg("Feed job_id does not match verify args.")]
    InvalidFeedJobId,
    #[msg("Could not derive group bitmap (timeout or invalid parameters).")]
    GroupBitmapDerivationFailed,
}
