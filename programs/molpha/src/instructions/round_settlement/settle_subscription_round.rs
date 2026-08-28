use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_request_auth_fields, Delegate, Gateway, ProtocolConfig, Registry, RegistryState,
        RoundReceiptInitArgs, RoundReceiptRecord, RoundRecordKind, Subscription,
    },
    utils::ed25519::verify_ed25519_signature,
};
use anchor_lang::prelude::*;

use super::validate_attestation::{validate_settlement_attestation, SettlementAttestationInput};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub timestamp: u64,
    pub source_id: [u8; 32],
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: crate::state::AggregatedSig,
    pub value: [u8; 32],
    pub consumer_authority: Pubkey,
}

pub fn settle_subscription_round<'info>(
    ctx: Context<SettleSubscriptionRound<'info>>,
    args: SettleRoundArgs,
) -> Result<()> {
    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let subscription = &mut ctx.accounts.subscription;
    let config = &ctx.accounts.protocol_config;
    let gateway = &mut ctx.accounts.gateway;
    let registry = ctx.accounts.registry.load()?;

    require!(
        args.signatures_required >= config.min_signers,
        MolphaError::SignaturesBelowMinSigners
    );
    require!(
        subscription.is_active_at(clock.unix_timestamp) && subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

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

    let validated = validate_settlement_attestation(
        &ctx.accounts.registry_state,
        &registry,
        SettlementAttestationInput {
            source_id: &args.source_id,
            canonical_timestamp: args.timestamp,
            signatures_required: args.signatures_required,
            registry_version: args.registry_version,
            value: &args.value,
            agg_sig: &args.agg_sig,
        },
    )?;

    let reward_per_signature = config.reward_per_signature;

    subscription.consume_round()?;

    ctx.accounts.round_record.init(RoundReceiptInitArgs {
        consumer_id: subscription.owner.to_bytes(),
        agg_sig: args.agg_sig,
        attestation_message_hash: validated.attestation_message_hash,
        gateway: gateway.key(),
        registry_version: args.registry_version,
        reward_per_signature,
        protocol_fee: 0,
        recorded_slot: now_slot,
        kind: RoundRecordKind::Subscription,
        bump: ctx.bumps.round_record,
    });

    emit!(RoundSettled {
        gateway: gateway.key(),
        source_id: args.source_id,
        timestamp: args.timestamp,
        signers_bitmap: ctx.accounts.round_record.signers_bitmap,
        credited_count: validated.credited_count,
        reward_per_signature,
        protocol_fee: 0,
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
pub struct SettleSubscriptionRound<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

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

    #[account(mut)]
    pub consumer_delegate: Option<Account<'info, Delegate>>,

    #[account(
        mut,
        has_one = authority,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
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
