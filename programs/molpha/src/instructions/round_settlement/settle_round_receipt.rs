use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_agg_sig, hash_request_auth_fields, resolve_registry_signers, AggregatedSig, Delegate,
        Feed, Gateway, ProtocolConfig, Registry, RegistryState, RoundReceiptRecord, Subscription,
    },
    utils::ed25519::verify_ed25519_signature,
};
use anchor_lang::prelude::*;
use molpha_verifier::bitmap::bitmap_load;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub timestamp: u64,
    pub source_id: [u8; 32],
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: AggregatedSig,
    pub value: [u8; 32],
    pub consumer_authority: Pubkey,
}

pub fn settle_round_receipt<'info>(
    ctx: Context<SettleRoundReceipt<'info>>,
    args: SettleRoundArgs,
) -> Result<()> {
    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let subscription = &ctx.accounts.subscription;
    let config = &ctx.accounts.protocol_config;
    let gateway = &mut ctx.accounts.gateway;

    let registry = ctx.accounts.registry.load()?;

    require!(
        subscription.is_active() && subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    require!(
        args.registry_version == ctx.accounts.registry_state.current_version
            && registry.version == args.registry_version,
        MolphaError::InvalidRegistryVersion
    );

    require!(
        args.value.len() <= Feed::MAX_VALUE_LEN,
        MolphaError::InvalidValueLength
    );

    // 3. Consumer authorization (owner-direct or delegate/agent).
    authorize_consumer(
        subscription,
        &mut ctx.accounts.consumer_delegate,
        args.consumer_authority,
        args.signatures_required,
    )?;

    let request_auth_hash =
        hash_request_auth_fields(&args.source_id, args.signatures_required, args.timestamp);
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &args.consumer_authority.to_bytes(),
            &request_auth_hash,
        )?,
        MolphaError::ConsumerSigInvalid
    );

    // 4. Quorum shape on the embedded bitmap (no selection re-derivation here).
    let credited = bitmap_load(&args.agg_sig.signers_bitmap);
    let credited_count = credited.count_ones();
    require!(
        credited_count >= u32::from(args.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        credited_count
            <= u32::from(args.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

    let agg_sig_hash = hash_agg_sig(&args.agg_sig);
    let attestation_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::AttestationPayload {
            value: args.value,
            source_id: args.source_id,
            signatures_required: args.signatures_required,
            canonical_timestamp: args.timestamp,
            registry_version: args.registry_version,
        },
        args.agg_sig.signers_bitmap,
    );

    // ---- Validate credited nodes; rewards are recorded on the round receipt only. ----
    let reward_per_signature = config.reward_per_signature;
    let gateway_fee = config.gateway_fee;
    resolve_registry_signers(
        ctx.remaining_accounts,
        ctx.program_id,
        &registry,
        &args.agg_sig.signers_bitmap,
    )?;

    ctx.accounts.subscription.consume_round()?;

    // 9 (dedup) is enforced by `init` on the record PDA.
    let record = &mut ctx.accounts.round_record;
    record.source_id = args.source_id;
    record.timestamp = args.timestamp;
    record.signatures_required = args.signatures_required;
    record.signers_bitmap = args.agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.attestation_message_hash = attestation_message_hash;
    record.gateway = gateway.key();
    record.registry_version = args.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.price = 0;
    record.bump = ctx.bumps.round_record;

    emit!(RoundSettled {
        gateway: gateway.key(),
        source_id: args.source_id,
        timestamp: args.timestamp,
        signers_bitmap: args.agg_sig.signers_bitmap,
        credited_count,
        reward_per_signature,
        gateway_fee,
        recorded_slot: now_slot,
    });

    Ok(())
}

fn authorize_consumer(
    subscription: &Account<'_, Subscription>,
    delegate: &mut Option<Account<'_, Delegate>>,
    consumer_authority: Pubkey,
    signatures_required: u8,
) -> Result<()> {
    if consumer_authority == subscription.owner {
        require!(delegate.is_none(), MolphaError::UnexpectedDelegateAccount);
        require!(
            signatures_required <= subscription.max_signers,
            MolphaError::SignaturesExceedSubscriptionMax
        );
        return Ok(());
    }

    let delegate = delegate
        .as_mut()
        .ok_or(MolphaError::DelegateAccountRequired)?;

    require_keys_eq!(
        delegate.owner,
        subscription.owner,
        MolphaError::UnauthorisedConsumer
    );
    require_keys_eq!(
        delegate.delegate,
        consumer_authority,
        MolphaError::UnauthorisedConsumer
    );
    require!(delegate.is_active(), MolphaError::DelegateNotActive);
    require!(
        signatures_required <= delegate.max_signers,
        MolphaError::SignaturesExceedDelegateMax
    );
    delegate.try_consume_data_request()?;

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SettleRoundArgs)]
pub struct SettleRoundReceipt<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(mut)]
    pub subscription: Account<'info, Subscription>,

    /// The handler checks the delegate PDA, owner/delegate fields, and discriminator before use.
    #[account(mut)]
    pub consumer_delegate: Option<Account<'info, Delegate>>,

    #[account(
        mut,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        init,
        payer = payer,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::SEED_PREFIX,
            subscription.owner.as_ref(),
            args.source_id.as_ref(),
            args.signatures_required.to_le_bytes().as_ref(),
            args.timestamp.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Account<'info, RoundReceiptRecord>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = solana_instructions_sysvar::id() @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
