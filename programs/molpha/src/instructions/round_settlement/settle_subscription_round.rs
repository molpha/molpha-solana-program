use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_request_auth_fields, Delegate, Gateway, GatewayStatus, ProtocolConfig, Registry,
        RoundReplayMarker, Subscription,
    },
    utils::{
        attestation::validate_attestation, ed25519::verify_ed25519_signature,
        settlement::credit_round_rewards,
    },
};
use molpha_verifier::Attestation;

pub fn settle_subscription_round<'info>(
    ctx: Context<'info, SettleSubscriptionRound<'info>>,
    attestation: Attestation,
    consumer_authority: Pubkey,
) -> Result<()> {
    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let subscription = &mut ctx.accounts.subscription;
    let gateway = &mut ctx.accounts.gateway;

    require!(
        subscription.is_active_at(clock.unix_timestamp) && subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    authorize_consumer(
        subscription,
        &mut ctx.accounts.consumer_delegate,
        consumer_authority,
        attestation.payload.signatures_required,
    )?;

    let request_auth_hash = hash_request_auth_fields(&gateway.key(), &attestation.payload);
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &consumer_authority.to_bytes(),
            &request_auth_hash,
        )?,
        MolphaError::ConsumerSigInvalid
    );

    validate_attestation(
        ctx.remaining_accounts,
        &ctx.accounts.registry,
        &ctx.accounts.config,
        &attestation,
        clock.unix_timestamp,
    )?;

    credit_round_rewards(
        &ctx.accounts.registry,
        &mut ctx.accounts.config,
        &ctx.accounts.treasury,
        ctx.remaining_accounts,
        &attestation,
        ctx.accounts.round_replay.key(),
    )?;

    subscription.consume_round()?;
    ctx.accounts.round_replay.bump = ctx.bumps.round_replay;

    emit!(RoundSettled {
        gateway: gateway.key(),
        source_id: attestation.payload.source_id,
        canonical_timestamp: attestation.payload.canonical_timestamp,
        signers_bitmap: attestation.signature.signers_bitmap,
        reward_per_signature: ctx.accounts.config.reward_per_signature,
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
    require!(
        signatures_required <= subscription.max_signers,
        MolphaError::SignaturesExceedSubscriptionMax
    );

    if consumer_authority == subscription.owner {
        require!(delegate.is_none(), MolphaError::UnexpectedDelegateAccount);
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
#[instruction(attestation: Attestation, consumer_authority: Pubkey)]
pub struct SettleSubscriptionRound<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = config.bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [Registry::SEED_PREFIX, attestation.payload.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(mut)]
    pub subscription: Account<'info, Subscription>,

    #[account(
        associated_token::mint = config.usdc_mint,
        associated_token::authority = config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [Delegate::SEED_PREFIX, subscription.owner.as_ref(), consumer_authority.as_ref()],
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
        space = RoundReplayMarker::SPACE,
        seeds = [
            RoundReplayMarker::SEED_PREFIX,
            subscription.owner.as_ref(),
            gateway.key().as_ref(),
            attestation.payload.source_id.as_ref(),
            attestation.payload.signatures_required.to_le_bytes().as_ref(),
            attestation.payload.canonical_timestamp.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub round_replay: Account<'info, RoundReplayMarker>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = solana_instructions_sysvar::id() @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
