use anchor_lang::prelude::*;
use molpha_verifier::DataUpdateError;

#[error_code]
pub enum MolphaError {
    #[msg("Registry change attempted while previous version is still accepted")]
    TransitionStillInGrace,
    #[msg("Signer index outside node_count(version)")]
    InvalidNodeIndex,
    #[msg("Node account does not match expected index/owner")]
    InvalidNodeAccount,
    #[msg("Previous-after-remove verification requires remove-transition metadata")]
    InvalidTransitionAccount,
    #[msg("moved_operator does not match the last-slot node's owner")]
    MovedNodeMismatch,
    #[msg("Caller is not node.owner or registry.authority")]
    UnauthorizedNodeAuthority,
    #[msg("secp256k1_pubkey matches existing active node")]
    NodeAlreadyRegistered,
    #[msg("pending_node_count >= max_node_count or node_index_hint >= max_node_count")]
    NodeCapExceeded,
    #[msg("PoP Schnorr verification failed at registration")]
    InvalidPoP,
    #[msg("Instruction requires Active node status")]
    NodeNotActive,
    #[msg("Submitted compressed pubkey does not decompress to a valid curve point")]
    PubkeyNotOnCurve,
    #[msg("registry_version is neither active_version nor grace-valid previous_version")]
    InvalidRegistryVersion,
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
    #[msg("remove_delegate target not found")]
    DelegateNotFound,
    #[msg("delegate is paused")]
    DelegateNotActive,
    #[msg("signatures_required exceeds delegate.max_signers")]
    SignaturesExceedDelegateMax,
    #[msg("subscription delegate_count >= plan.max_delegates")]
    DelegateCapReached,
    #[msg("registration_salt must not be zero")]
    InvalidRegistrationSalt,
    #[msg("secp256k1 key fingerprint does not match compressed key")]
    InvalidSecp256k1KeyFingerprint,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    InsufficientSigners,
    #[msg("signatures_required exceeds subscription.max_signers")]
    SignaturesExceedSubscriptionMax,
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

    // ---- Receipt settlement (v0.2). Numeric codes are sequential from the macro; the
    //      spec's 62xx values are illustrative. Names match the spec §9. ----
    #[msg("consumer Ed25519 signature missing or invalid")]
    ConsumerSigInvalid,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    QuorumBelowThreshold,
    #[msg("gateway not registered or bonded below gateway_bond_min")]
    GatewayNotBonded,
    #[msg("subscription inactive or round quota exhausted")]
    SubscriptionInactiveOrExhausted,
    #[msg("supplied signature does not match committed agg_sig_hash")]
    DisputeAggSigMismatch,
    #[msg("dispute failed: the committed signature verifies (no fraud)")]
    DisputeSignatureValid,
    #[msg("gateway_index does not match the bonded gateway")]
    GatewayIndexMismatch,
    #[msg("bond amount below gateway_bond_min")]
    InsufficientBond,
    #[msg("agg_sig.signers_bitmap does not match signers_bitmap")]
    CreditedBitmapMismatch,
    #[msg("round still inside dispute window; cannot finalize")]
    RoundNotFinalizable,
    #[msg("round is not pending")]
    RoundNotPending,
    #[msg("no claimable rewards")]
    NothingToClaim,
    #[msg("instructions sysvar account is invalid")]
    InvalidInstructionsSysvar,
    #[msg("consumer authority is not the delegate authority")]
    UnauthorisedConsumer,
    #[msg("mismatched or missing credited node account")]
    InvalidCreditNodeAccount,

    // ---- Agent pay-per-request access (x402 / self-funded). ----
    #[msg("agent escrow is not active")]
    AgentNotActive,
    #[msg("computed price does not match the agent-signed amount")]
    AmountMismatch,
    #[msg("arithmetic overflow in escrow spend accounting")]
    EpochCapExceeded,
    #[msg("escrow ATA balance below computed price")]
    InsufficientAgentCredits,
    #[msg("agent request authorization signature is invalid or missing")]
    AgentSigInvalid,
    #[msg("agent cap parameters invalid")]
    InvalidAgentCaps,
    #[msg("credited signer count exceeds signatures_required + redundancy_buffer")]
    CreditedExceedsSelection,
    #[msg("value exceeds Feed::MAX_VALUE_LEN")]
    InvalidValueLength,
}

pub(crate) fn map_data_update_err(error: DataUpdateError) -> anchor_lang::error::Error {
    match error {
        DataUpdateError::InvalidAggregateSignature => {
            error!(MolphaError::InvalidAggregateSignature)
        }
        DataUpdateError::InvalidSignature => error!(MolphaError::InvalidSignature),
        DataUpdateError::InsufficientSigners => error!(MolphaError::InsufficientSigners),
        DataUpdateError::SignersNotSubsetOfSelection => {
            error!(MolphaError::SignersNotSubsetOfSelection)
        }
        DataUpdateError::InvalidSignersBitmap => error!(MolphaError::InvalidSignersBitmap),
        DataUpdateError::GroupBitmapDerivationFailed => {
            error!(MolphaError::GroupBitmapDerivationFailed)
        }
        DataUpdateError::SignerCountMismatch | DataUpdateError::MissingSignerAccount => {
            error!(MolphaError::MissingSignerAccount)
        }
        DataUpdateError::InvalidRegistryVersion => error!(MolphaError::InvalidRegistryVersion),
        DataUpdateError::InvalidNodeIndex => error!(MolphaError::InvalidNodeIndex),
        DataUpdateError::InvalidTransitionAccount => {
            error!(MolphaError::InvalidTransitionAccount)
        }
    }
}
