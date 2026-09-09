use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_request_auth_fields, AggregatedSig, Delegate, Gateway, GatewayStatus, ProtocolConfig, Registry,
        RoundReceiptInitArgs, RoundReceiptRecord, RoundRecordKind, Subscription,
    },
    utils::{
        ed25519::verify_ed25519_signature,
        settlement::{validate_settlement_attestation, RoundRequest},
    },
};
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub agg_sig: AggregatedSig,
    pub value: [u8; 32],
    pub consumer_authority: Pubkey,
}

pub fn settle_subscription_round<'info>(
    ctx: Context<SettleSubscriptionRound<'info>>,
    round_request: RoundRequest,
    args: SettleRoundArgs,
) -> Result<()> {
    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let subscription = &mut ctx.accounts.subscription;
    let config = &ctx.accounts.protocol_config;
    let gateway = &mut ctx.accounts.gateway;
    let registry = ctx.accounts.registry.load()?;

    require!(
        subscription.is_active_at(clock.unix_timestamp) && subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    authorize_consumer(
        subscription,
        &mut ctx.accounts.consumer_delegate,
        args.consumer_authority,
        round_request.signatures_required,
    )?;

    let request_auth_hash = hash_request_auth_fields(
        &gateway.key(),
        &round_request,
    );
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &args.consumer_authority.to_bytes(),
            &request_auth_hash,
        )?,
        MolphaError::ConsumerSigInvalid
    );

    let validated = validate_settlement_attestation(
        &registry,
        &config,
        &round_request,
        &args.agg_sig,
        args.value,
        clock.unix_timestamp,
    )?;

    let reward_per_signature = config.reward_per_signature;

    subscription.consume_round()?;

    ctx.accounts.round_record.init(RoundReceiptInitArgs {
        consumer_id: subscription.owner.to_bytes(),
        selection_bitmap: validated.selection_bitmap,
        agg_sig: args.agg_sig,
        attestation_message_hash: validated.attestation_message_hash,
        gateway: gateway.key(),
        registry_version: round_request.registry_version,
        reward_per_signature,
        recorded_slot: now_slot,
        kind: RoundRecordKind::Subscription,
        bump: ctx.bumps.round_record,
    });

    emit!(RoundSettled {
        gateway: gateway.key(),
        source_id: round_request.source_id,
        canonical_timestamp: round_request.canonical_timestamp,
        credited_count: validated.credited_count,
        reward_per_signature,
        recorded_slot: now_slot,
        recorded_at: clock.unix_timestamp,
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
    delegate.try_consume_data_request()?;

    Ok(())
}

#[derive(Accounts)]
#[instruction(round_request: RoundRequest, args: SettleRoundArgs)]
pub struct SettleSubscriptionRound<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [Registry::SEED_PREFIX, round_request.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(mut)]
    pub subscription: Account<'info, Subscription>,

    #[account(
        mut,
        seeds = [Delegate::SEED_PREFIX, subscription.owner.as_ref(), args.consumer_authority.as_ref()],
        bump = consumer_delegate.bump,
    )]
    pub consumer_delegate: Option<Account<'info, Delegate>>,

    #[account(
        mut,
        has_one = authority,
        constraint = gateway.status == GatewayStatus::Active @ MolphaError::GatewayNotActive,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        init,
        payer = authority,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::SEED_PREFIX,
            subscription.owner.as_ref(),
            gateway.key().as_ref(),
            round_request.source_id.as_ref(),
            round_request.signatures_required.to_le_bytes().as_ref(),
            round_request.canonical_timestamp.to_le_bytes().as_ref(),
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
