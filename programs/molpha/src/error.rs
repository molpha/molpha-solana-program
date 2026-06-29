use anchor_lang::prelude::*;
use molpha_verifier::DataUpdateError;

#[error_code]
pub enum MolphaError {
    #[msg("Registry change attempted while previous version is still accepted")]
    TransitionStillInGrace,
    #[msg("`previous_version` used after previous_expires_at")]
    PreviousRegistryExpired,
    #[msg("Signer index outside node_count(version)")]
    InvalidNodeIndex,
    #[msg("RegistryIndex does not match expected index/node")]
    InvalidRegistryIndexAccount,
    #[msg("Previous-after-remove verification requires remove-transition metadata")]
    InvalidTransitionAccount,
    #[msg("moved_node.node_id does not match last_index_account.node_id")]
    MovedNodeMismatch,
    #[msg("new_index must equal registry.current_node_count")]
    InvalidNewIndex,
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
    #[msg("security_deposit < minimum_node_deposit")]
    InsufficientDeposit,
    #[msg("Submitted compressed pubkey does not decompress to a valid curve point")]
    PubkeyNotOnCurve,
    #[msg("registry_version is neither active_version nor grace-valid previous_version")]
    InvalidRegistryVersion,
    #[msg("Pending registry version used before activation")]
    VersionNotActive,
    #[msg("Supplied NodeSlotVersionAccount does not cover registry_version")]
    InvalidNodeSlotVersion,
    #[msg("Attempted slot reuse before reusable_after")]
    NodeSlotNotReusable,
    #[msg("Caller is not ProtocolConfig.authority")]
    UnauthorisedAuthority,
    #[msg("Gateway endpoint is invalid")]
    InvalidGatewayEndpoint,
    #[msg("node_identity.owner does not match signer")]
    UnauthorisedNodeOwner,
    #[msg("node_slot.current_node_id does not match node_identity.node_id")]
    NodeSlotIdentityMismatch,
    #[msg("Pending registry version is not ready for activation")]
    PendingVersionNotReady,
    #[msg("No pending registry fork to activate")]
    NoPendingRegistryFork,
    #[msg("node_slot_version interval does not match expected from_version")]
    InvalidSlotVersionInterval,
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
    #[msg("job_id PDA already initialised")]
    JobAlreadyExists,
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
    #[msg("delegate jobs_created >= max_jobs_created")]
    DelegateJobLimitReached,
    #[msg("delegate data_requests_used >= max_data_requests")]
    DelegateDataRequestLimitReached,
    #[msg("remove_delegate target not found")]
    DelegateNotFound,
    #[msg("delegate PDA already exists for this owner/delegate pair")]
    DelegateAlreadyPresent,
    #[msg("job.owner does not match signer")]
    UnauthorisedJobOwner,
    #[msg("job_id does not match derived value for args and created_at_slot")]
    InvalidJobId,
    #[msg("node_id does not match derived value for owner, pubkey, and salt")]
    InvalidNodeId,
    #[msg("Re-derived selection bitmap does not match submitted bitmap")]
    InvalidSelectionBitmap,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    InsufficientSigners,
    #[msg("signatures_required exceeds plan.max_signers")]
    SignaturesExceedPlanMax,
    #[msg("Signer bits outside selection bitmap")]
    SignersNotSubsetOfSelection,
    #[msg("PoP-Schnorr aggregate verification failed")]
    InvalidAggregateSignature,
    #[msg("canonical_timestamp <= feed.canonical_timestamp")]
    FeedNotNewer,
    #[msg("args.job_id does not match job.job_id")]
    DataUpdateJobMismatch,
    #[msg("api_config_hash mismatch")]
    DataUpdateConfigMismatch,
    #[msg("Missing or mis-ordered signer slot version account")]
    MissingSignerAccount,
    #[msg("subscription has expired")]
    SubscriptionNotActive,
    #[msg("subscription job_count >= plan.max_jobs")]
    JobLimitReached,

    // ---- Receipt settlement (v0.2). Numeric codes are sequential from the macro; the
    //      spec's 62xx values are illustrative. Names match the spec §9. ----
    #[msg("consumer Ed25519 signature missing or invalid")]
    ConsumerSigInvalid,
    #[msg("request_auth.timestamp must be strictly greater than job.last_request_timestamp")]
    TimestampNotIncreasing,
    #[msg("re-derived selection bitmap does not match the gateway receipt")]
    SelectionBitmapMismatch,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    QuorumBelowThreshold,
    #[msg("args.job_id != job.job_id")]
    AggSigJobMismatch,
    #[msg("gateway Ed25519 signature missing or invalid")]
    GatewaySigInvalid,
    #[msg("gateway not registered or bonded below gateway_bond_min")]
    GatewayNotBonded,
    #[msg("(job_id, timestamp) receipt already recorded")]
    ReceiptAlreadyRecorded,
    #[msg("subscription inactive or round quota exhausted")]
    SubscriptionInactiveOrExhausted,
    #[msg("supplied signature does not match committed agg_sig_hash")]
    DisputeAggSigMismatch,
    #[msg("dispute window has closed; round is finalized")]
    DisputeWindowClosed,
    #[msg("dispute failed: the committed signature verifies (no fraud)")]
    DisputeSignatureValid,
    #[msg("gateway_index does not match the bonded gateway")]
    GatewayIndexMismatch,
    #[msg("gateway authority already registered")]
    GatewayAlreadyRegistered,
    #[msg("bond amount below gateway_bond_min")]
    InsufficientBond,
    #[msg("agg_sig.signers_bitmap does not match credited_bitmap")]
    CreditedBitmapMismatch,
    #[msg("round still inside dispute window; cannot finalize")]
    RoundNotFinalizable,
    #[msg("round already finalized")]
    RoundAlreadyFinalized,
    #[msg("round was voided by a successful dispute")]
    RoundVoided,
    #[msg("no claimable rewards")]
    NothingToClaim,
    #[msg("instructions sysvar account is invalid")]
    InvalidInstructionsSysvar,
    #[msg("consumer authority is not the job owner or a delegate")]
    UnauthorisedConsumer,
    #[msg("mismatched or missing credited node account")]
    InvalidCreditNodeAccount,
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
