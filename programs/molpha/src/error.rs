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
    #[msg("registry version counter overflow")]
    RegistryVersionOverflow,
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
    #[msg("source_id does not match feed.source_id")]
    InvalidSourceId,
    #[msg("new redundancy buffer must differ from the current registry policy")]
    RegistryPolicyUnchanged,
    #[msg("Caller is not ProtocolConfig.authority")]
    UnauthorisedAuthority,
    #[msg("Gateway endpoint is invalid")]
    InvalidGatewayEndpoint,
    #[msg("Node endpoint is invalid")]
    InvalidNodeEndpoint,
    #[msg("treasury token account mismatch")]
    InvalidTreasury,
    #[msg("owner_usdc token account mismatch")]
    InvalidOwnerUsdc,
    #[msg("treasury must be owned by protocol_config")]
    InvalidTreasuryOwner,
    #[msg("withdraw destination token account mismatch")]
    InvalidWithdrawDestination,
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
    #[msg("delegate is paused")]
    DelegateNotActive,
    #[msg("signatures_required exceeds delegate.max_signers")]
    SignaturesExceedDelegateMax,
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
    #[msg("gateway is deactivated")]
    GatewayDeactivated,
    #[msg("gateway is slashed")]
    GatewaySlashed,
    #[msg("gateway is active")]
    GatewayActive,
    #[msg("subscription inactive or round quota exhausted")]
    SubscriptionInactiveOrExhausted,
    #[msg("dispute failed: the committed signature verifies (no fraud)")]
    DisputeSignatureValid,
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
    #[msg("round dispute window has elapsed")]
    DisputeWindowElapsed,
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
    #[msg("every signer was within the derived selection - no unsolicited signer")]
    SignersFullySelected,
    #[msg("every guilty node is already tombstoned")]
    NothingToPunish,
    #[msg("statements do not share an aggregate nonce commitment")]
    NonceCommitmentMismatch,
    #[msg("statements commit to the same message hash")]
    NonceReuseSameMessage,
    #[msg("same duty with different values is equivocation - file it as one")]
    PreferEquivocation,
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
