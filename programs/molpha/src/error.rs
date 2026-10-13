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
    #[msg("subscription delegate_count >= plan.max_delegates")]
    DelegateCapReached,
    #[msg("registration_salt must not be zero")]
    InvalidRegistrationSalt,
    #[msg("popcount(signers_bitmap) < signatures_required")]
    InsufficientSigners,
    #[msg("signatures_required is below protocol min_signers")]
    SignaturesBelowMinSigners,
    #[msg("Signer bits outside selection bitmap")]
    SignersNotSubsetOfSelection,
    #[msg("PoP-Schnorr aggregate verification failed")]
    InvalidAggregateSignature,
    #[msg("timestamp <= feed.timestamp")]
    FeedNotNewer,
    #[msg("signatures_required does not match feed.signatures_required")]
    SignaturesRequiredMismatch,
    #[msg("Missing or mis-ordered signer slot version account")]
    MissingSignerAccount,
    #[msg("subscription has expired")]
    SubscriptionNotActive,

    #[msg("popcount(signers_bitmap) < signatures_required")]
    QuorumBelowThreshold,
    #[msg("gateway is not active")]
    GatewayNotActive,
    #[msg("no claimable rewards")]
    NothingToClaim,
    #[msg("account is not in a state that permits withdrawal")]
    InvalidWithdrawalState,
    #[msg("withdrawable balance is zero")]
    NothingToWithdraw,
    #[msg("withdrawal cooldown has not elapsed")]
    WithdrawalCooldownActive,
    #[msg("withdrawal deadline arithmetic overflow")]
    WithdrawalArithmeticOverflow,
    #[msg("withdrawal cooldown must be positive and fit a slot deadline")]
    InvalidWithdrawalCooldown,
    #[msg("instructions sysvar account is invalid")]
    InvalidInstructionsSysvar,

    #[msg("credited signer count exceeds signatures_required + redundancy_buffer")]
    CreditedExceedsSelection,
    #[msg(
        "raw value exceeds Feed::MAX_VALUE_LEN or does not match the hash of the attestation value"
    )]
    InvalidRawValue,

    #[msg("equivocation requires two attestations for one duty with different values")]
    EquivocationSameStatement,
    #[msg("attestations do not sign different values for one duty")]
    NotEquivocation,
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
    #[msg("attestations do not share an aggregate nonce commitment")]
    NonceCommitmentMismatch,
    #[msg("attestations have different registry versions, or one older than the feed's")]
    RegistryVersionMismatch,
    #[msg("attestations sign the same message")]
    NonceReuseSameMessage,
    #[msg("same duty with different values is equivocation - file it as one")]
    PreferEquivocation,
    #[msg("timestamp is older than now - max_settlement_delay_seconds")]
    AttestationTooStale,
    #[msg("timestamp is further ahead of cluster time than the allowed drift")]
    AttestationInFuture,
    #[msg("max_settlement_delay_seconds must be positive and fit a timestamp deadline")]
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
    #[msg("minimum node deposit must be positive")]
    InvalidMinimumNodeDeposit,
    #[msg("base price plus rewards for the maximum signer count must fit u64")]
    InvalidRoundPrice,
    #[msg("reward or reward liability arithmetic overflowed")]
    RewardOverflow,
    #[msg("signer account list must contain exactly one account per signer")]
    UnexpectedSignerAccounts,
    #[msg("coalition key is not the sum of the signer keys")]
    InvalidCoalitionKey,

    #[msg("epoch_len_seconds is outside the allowed bounds")]
    InvalidEpochLength,
    #[msg("ticket_grace_seconds must be positive and shorter than the epoch")]
    InvalidTicketGrace,
    #[msg("target_tickets_per_epoch is outside the allowed bounds")]
    InvalidTicketTarget,
    #[msg("availability floor must not exceed 10000 bps and needs a positive sample minimum")]
    InvalidAvailabilityFloor,
    #[msg("protocol_fee_bps must not exceed 10000")]
    InvalidProtocolFee,
    #[msg("epoch is not open at the current cluster time")]
    EpochNotOpen,
    #[msg("epoch no longer accepts tickets")]
    TicketWindowClosed,
    #[msg("timestamp is outside the epoch window")]
    RoundOutsideEpoch,
    #[msg("round does not fall under the epoch ticket threshold")]
    NotATicket,
    #[msg("gateway dispatch signature missing or invalid")]
    DispatchSigInvalid,
    #[msg("roster slot hint does not match the epoch roster")]
    InvalidRosterSlot,
    #[msg("epoch ticket cap reached")]
    TicketCapReached,
    #[msg("ticket marker is still inside its epoch ticket window")]
    TicketMarkerNotExpired,
    #[msg("epoch ticket window has not ended")]
    EpochNotFinalizable,
    #[msg("epoch is not finalized")]
    EpochNotFinalized,
    #[msg("next epoch account does not follow the finalized epoch")]
    InvalidEpochSequence,
    #[msg("roster slot has no weight or was already credited")]
    NothingToCredit,
    #[msg("epoch still has uncredited nodes or is inside its evidence window")]
    EpochNotClosable,
    #[msg("treasury holds no unswept income")]
    NothingToSweep,
    #[msg("amount exceeds the protocol reserve")]
    InsufficientProtocolReserve,
    #[msg("node availability is not below the epoch payout floor")]
    AvailabilityAboveFloor,
    #[msg("node was already frozen during this epoch")]
    InactivityAlreadyPunished,
    #[msg("rent receiver must be the account that paid for the closed account")]
    InvalidRentReceiver,
    #[msg("initial_tau is below the minimum ticket threshold")]
    InvalidInitialTau,
    #[msg("program data account is not this program's")]
    InvalidProgramData,
    #[msg("initialize must be signed by the program's upgrade authority")]
    NotUpgradeAuthority,
    #[msg("the node's current signing key did not sign this rotation")]
    InvalidKeyRotationSignature,
    #[msg("the node already rotated its signing key within the settlement window")]
    KeyRotationCooldownActive,
    #[msg("the new signing key is the node's current key")]
    KeyRotationKeyUnchanged,
    #[msg("the next registry version is too large to allocate in one instruction; call extend_registry first")]
    RegistryNotAllocated,
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
        AttestationError::InvalidRegistryVersion => error!(MolphaError::InvalidRegistryVersion),
        AttestationError::InvalidNodeIndex => error!(MolphaError::InvalidNodeIndex),
        AttestationError::InvalidCoalitionKey => error!(MolphaError::InvalidCoalitionKey),
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
    }
}
