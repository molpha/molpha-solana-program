use anchor_lang::prelude::*;
use molpha_verifier::{solana::AccountError, AttestationError};

#[error_code]
pub enum MolphaError {
    #[msg("invalid min signers")]
    InvalidMinSigners,
    #[msg("invalid timestamp")]
    InvalidTimestamp,
    #[msg("invalid usdc mint")]
    InvalidMint,
    #[msg("Signer index outside node_count(version)")]
    InvalidNodeIndex,
    #[msg("Node account does not match expected index/owner")]
    InvalidNodeAccount,
    #[msg("Caller is not node.owner or registry.authority")]
    UnauthorizedNodeAuthority,
    #[msg("pending_node_count >= registry.node_count or node_index_hint >= registry.node_count")]
    NodeCapExceeded,
    #[msg("PoP Schnorr verification failed at registration")]
    InvalidPoP,
    #[msg("Instruction requires Active node status")]
    NodeNotActive,
    #[msg("Submitted compressed pubkey does not decompress to a valid curve point")]
    PubkeyNotOnCurve,
    #[msg("registry version or version-addressed registry account is invalid")]
    InvalidRegistryVersion,
    #[msg("registry is not active")]
    RegistryNotActive,
    #[msg("source_id does not match feed.source_id")]
    InvalidSourceId,
    #[msg("new redundancy buffer must differ from the current registry policy")]
    RegistryPolicyUnchanged,
    #[msg("redundancy buffer must be between 1 and 8")]
    InvalidRedundancyBuffer,
    #[msg("Gateway endpoint is invalid")]
    InvalidGatewayEndpoint,
    #[msg("Node endpoint is invalid")]
    InvalidNodeEndpoint,
    #[msg("owner_usdc token account mismatch")]
    InvalidOwnerUsdc,
    #[msg("Invalid aggregate signature")]
    InvalidSignature,
    #[msg("Could not derive group bitmap")]
    GroupBitmapDerivationFailed,
    #[msg("Signers bitmap contains out-of-range indices")]
    InvalidSignersBitmap,
    #[msg("plan exists but is_active is false")]
    PlanNotActive,
    #[msg("invalid plan quorum or signer parameters")]
    InvalidPlanParams,
    #[msg("invalid delegate limit parameters")]
    InvalidDelegateLimits,
    #[msg("delegate cannot be the owner")]
    DelegateCannotBeOwner,
    #[msg("delegate account required for non-owner action")]
    DelegateAccountRequired,
    #[msg("delegate account must not be supplied for owner action")]
    UnexpectedDelegateAccount,
    #[msg("delegate data_requests_used >= max_data_requests")]
    DelegateDataRequestLimitReached,
    #[msg("subscription delegate_count >= plan.max_delegates")]
    DelegateCapReached,
    #[msg("registration_salt must not be zero")]
    InvalidRegistrationSalt,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    InsufficientSigners,
    #[msg("signatures_required exceeds subscription.max_signers")]
    SignaturesExceedSubscriptionMax,
    #[msg("signatures_required is below protocol min_signers")]
    SignaturesBelowMinSigners,
    #[msg("Signer bits outside selection bitmap")]
    SignersNotSubsetOfSelection,
    #[msg("PoP-Schnorr aggregate verification failed")]
    InvalidAggregateSignature,
    #[msg("canonical_timestamp <= feed.canonical_timestamp")]
    FeedNotNewer,
    #[msg("Missing or mis-ordered signer slot version account")]
    MissingSignerAccount,
    #[msg("subscription has expired")]
    SubscriptionNotActive,

    #[msg("consumer Ed25519 signature missing or invalid")]
    ConsumerSigInvalid,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    QuorumBelowThreshold,
    #[msg("gateway is slashed")]
    GatewaySlashed,
    #[msg("gateway is not deactivated")]
    GatewayNotDeactivated,
    #[msg("gateway is not active")]
    GatewayNotActive,
    #[msg("caller is not gateway.authority or protocol_config.authority")]
    UnauthorizedGatewayAuthority,
    #[msg("subscription inactive or round quota exhausted")]
    SubscriptionInactiveOrExhausted,
    #[msg("gateway_index does not match the bonded gateway")]
    GatewayIndexMismatch,
    #[msg("bond amount below gateway_bond_min")]
    InsufficientBond,
    #[msg("round still inside dispute window; cannot finalize")]
    RoundNotFinalizable,
    #[msg("round is not pending")]
    RoundNotPending,
    #[msg("no claimable rewards")]
    NothingToClaim,
    #[msg("account is not in a state that permits withdrawal")]
    InvalidWithdrawalState,
    #[msg("withdrawable balance is zero")]
    NothingToWithdraw,
    #[msg("withdrawal cooldown has not elapsed")]
    WithdrawalCooldownActive,
    #[msg("gateway bond is still inside its dispute window")]
    GatewayDisputeWindowActive,
    #[msg("withdrawal deadline arithmetic overflow")]
    WithdrawalArithmeticOverflow,
    #[msg("withdrawal cooldown must be non-negative")]
    InvalidWithdrawalCooldown,
    #[msg("instructions sysvar account is invalid")]
    InvalidInstructionsSysvar,
    #[msg("consumer authority is not the delegate authority")]
    UnauthorisedConsumer,
    #[msg("mismatched or missing credited node account")]
    InvalidCreditNodeAccount,
    #[msg("mismatched or missing missed node account")]
    InvalidMissedNodeAccount,

    #[msg("computed price does not match the gateway-signed amount")]
    AmountMismatch,
    #[msg("gateway USDC float balance below computed price")]
    InsufficientGatewayFloat,
    #[msg("payment_ref exceeds 64 bytes")]
    PaymentRefTooLong,
    #[msg("credited signer count exceeds signatures_required + redundancy_buffer")]
    CreditedExceedsSelection,
    #[msg("value exceeds Feed::MAX_VALUE_LEN")]
    InvalidValueLength,
    #[msg("dispute refund/restore subscription does not match the round")]
    WrongSubscriptionAccount,

    #[msg("equivocation requires two statements with different values")]
    EquivocationSameStatement,
    #[msg("recomputed evidence hash does not match the supplied evidence_hash")]
    EvidenceHashMismatch,
    #[msg("intersection of the two signer bitmaps is empty")]
    GuiltySetEmpty,
    #[msg("node is permanently tombstoned")]
    NodeTombstoned,
    #[msg("node is not Frozen")]
    NodeNotFrozen,
    #[msg("node is still inside its freeze window")]
    NodeStillFrozen,
    #[msg("node is already present in the current registry")]
    NodeAlreadyInRegistry,
    #[msg("node liveliness score is not below the freeze threshold")]
    LivelinessAboveThreshold,
    #[msg("every signer was within the derived selection - no unsolicited signer")]
    SignersFullySelected,
    #[msg("every guilty node is already tombstoned")]
    NothingToPunish,
    #[msg("statements do not share an aggregate nonce commitment")]
    NonceCommitmentMismatch,
    #[msg("round requests have different registry versions")]
    RegistryVersionMismatch,
    #[msg("statements commit to the same message hash")]
    NonceReuseSameMessage,
    #[msg("same duty with different values is equivocation - file it as one")]
    PreferEquivocation,
    #[msg("canonical_timestamp is older than now - max_settlement_delay_seconds")]
    AttestationTooStale,
    #[msg("canonical_timestamp is further ahead of cluster time than the allowed drift")]
    AttestationInFuture,
    #[msg("max_settlement_delay_seconds must be greater than zero")]
    InvalidMaxReceiptDelay,
    #[msg("treasury balance cannot back the protocol's reward liability")]
    TreasuryInsolvent,
    #[msg("challenger_bounty_bps must not exceed 10000")]
    InvalidChallengerBounty,
    #[msg("reward liability underflow")]
    RewardLiabilityUnderflow,
    #[msg("node deposit vault does not cover the recorded total deposits")]
    DepositVaultUnderfunded,
    #[msg("both transcripts carry the same (b, c) challenge pair")]
    TranscriptContextsEqual,
    #[msg("scalar is zero or not below the secp256k1 curve order")]
    NonCanonicalScalar,
    #[msg("merkle proof exceeds the maximum accepted depth")]
    MerkleProofTooDeep,
    #[msg("merkle proof does not connect the leaf to the signed batch root")]
    InvalidMerkleProof,
    #[msg("commitment batch signature does not verify against the node key")]
    InvalidBatchSignature,
    #[msg("partial signature does not satisfy the nonce equation")]
    InvalidPartialSignature,
    #[msg("committed nonce is not a valid secp256k1 point")]
    InvalidNoncePoint,
}

pub(crate) fn map_attestation_err(error: AttestationError) -> anchor_lang::error::Error {
    match error {
        AttestationError::InvalidAggregateSignature => {
            error!(MolphaError::InvalidAggregateSignature)
        }
        AttestationError::InvalidSignature => error!(MolphaError::InvalidSignature),
        AttestationError::InsufficientSigners => error!(MolphaError::InsufficientSigners),
        AttestationError::SignersNotSubsetOfSelection => {
            error!(MolphaError::SignersNotSubsetOfSelection)
        }
        AttestationError::InvalidSignersBitmap => error!(MolphaError::InvalidSignersBitmap),
        AttestationError::GroupBitmapDerivationFailed => {
            error!(MolphaError::GroupBitmapDerivationFailed)
        }
        AttestationError::SignerCountMismatch | AttestationError::MissingSignerAccount => {
            error!(MolphaError::MissingSignerAccount)
        }
        AttestationError::InvalidRegistryVersion => error!(MolphaError::InvalidRegistryVersion),
        AttestationError::InvalidNodeIndex => error!(MolphaError::InvalidNodeIndex),
    }
}

pub(crate) fn map_attestation_accounts_err(error: AccountError) -> anchor_lang::error::Error {
    match error {
        AccountError::Attestation(inner) => map_attestation_err(inner),
        AccountError::AccountBorrowFailed | AccountError::InvalidAccountOwner => {
            error!(MolphaError::MissingSignerAccount)
        }
        AccountError::InvalidRegistryAccount | AccountError::InvalidRegistryPda => {
            error!(MolphaError::InvalidRegistryVersion)
        }
        AccountError::InvalidNodeAccount | AccountError::InvalidNodePda => {
            error!(MolphaError::InvalidNodeAccount)
        }
    }
}
