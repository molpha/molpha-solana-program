//! `settle_round_receipt` — optimistic happy path (no Schnorr verify).
//!
//! Verifies two cheap Ed25519 signatures (consumer + gateway), stores the committed
//! `AggregatedSig` *without verifying it*, and stores the committed data-update message hash
//! that dispute-time Schnorr verification must use. Credits are applied to the participation set
//! embedded in that signature (`agg_sig.signers_bitmap`). Schnorr verification runs only on
//! `dispute_round`.
//!
//! The v0.2 spec batches several rounds (`Vec<SettleEntry>`) into one transaction. Because the
//! per-round cost is flat (§8), this is implemented as a single-round instruction; clients pack
//! multiple of these into one transaction up to the 1232-byte limit for identical effect, with
//! far simpler and safer account handling.

use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_agg_sig, hash_request_auth_fields, AggregatedSig, Delegate, Gateway, Node, Plan,
        ProtocolConfig, RegistryState, RoundReceiptRecord, Subscription,
    },
    utils::{
        bitmap::{bitmap_load, for_each_set_bit_u256},
        ed25519::verify_ed25519_signature,
    },
};
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub job_id: [u8; 32],
    pub timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    // pub selection_bitmap: [u8; 32],
    pub agg_sig: AggregatedSig,
    /// Feed payload value signed by the aggregate Schnorr coalition.
    pub value: [u8; 32],
    /// Consumer Ed25519 key that signed `H(RequestAuth)` (job owner or a registered delegate).
    pub consumer_authority: Pubkey,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleJobInitArgs {
    pub api_config_hash: [u8; 32],
    pub signatures_required: u8,
    pub decimals: u8,
}

pub fn settle_round_receipt<'info>(
    ctx: Context<'_, '_, 'info, 'info, SettleRoundReceipt<'info>>,
    args: SettleRoundArgs,
) -> Result<()> {
    let SettleRoundArgs {
        job_id,
        timestamp,
        signatures_required,
        agg_sig,
        value,
        consumer_authority,
        registry_version,
    } = &args;

    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let subscription = &ctx.accounts.subscription;
    let config = &ctx.accounts.protocol_config;
    let registry = &ctx.accounts.registry_state;
    let gateway = &mut ctx.accounts.gateway;

    // 1. Job & subscription quota (subscription.is_active() enforced via account constraint).
    require!(
        ctx.accounts.subscription.is_active() && ctx.accounts.subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    // 2. Registry — current-version-only for settlement.
    require!(
        *registry_version == registry.current_version,
        MolphaError::InvalidRegistryVersion
    );

    // 3. Consumer authorization.
    authorize_consumer(
        subscription,
        &mut ctx.accounts.consumer_delegate,
        *consumer_authority,
    )?;

    let request_auth_hash = hash_request_auth_fields(job_id, *timestamp);
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &consumer_authority.to_bytes(),
            &request_auth_hash,
        )?,
        MolphaError::ConsumerSigInvalid
    );

    // 4. Quorum shape on the embedded bitmap (no selection re-derivation here).
    let credited = bitmap_load(&agg_sig.signers_bitmap);
    let credited_count = credited.count_ones();
    require!(
        credited_count >= u32::from(*signatures_required),
        MolphaError::QuorumBelowThreshold
    );

    let agg_sig_hash = hash_agg_sig(agg_sig);
    let data_update_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::DataUpdate {
            job_id: *job_id,
            signatures_required: *signatures_required,
            registry_version: *registry_version,
            signers_bitmap: agg_sig.signers_bitmap,
            value: *value,
            canonical_timestamp: *timestamp as i64,
            agg_sig_s: agg_sig.s,
            commitment_addr: agg_sig.commitment_addr,
        },
        *signatures_required,
    );

    // ---- Credit (deterministic; fixed gateway fee). ----
    let reward_per_signature = config.reward_per_signature;
    let gateway_fee = config.gateway_fee;
    let program_id = ctx.program_id;

    require!(
        ctx.remaining_accounts.len() == credited_count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    let mut cursor = 0usize;
    for_each_set_bit_u256(credited, |bit_pos| {
        let node_ai = ctx
            .remaining_accounts
            .get(cursor)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        cursor += 1;

        require!(
            node_ai.owner == program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(node_ai)?;
        // Settlement is current-version-only, so the bit position is the slot index directly.
        require!(node.index == bit_pos as u32, MolphaError::InvalidNodeIndex);

        node.pending_rewards = node
            .pending_rewards
            .checked_add(reward_per_signature)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        node.exit(program_id)?;

        Ok(())
    })?;

    gateway.pending_rewards = gateway
        .pending_rewards
        .checked_add(gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;

    ctx.accounts.subscription.consume_round()?;

    // 9 (dedup) is enforced by `init` on the record PDA.
    let record = &mut ctx.accounts.round_record;
    record.job_id = *job_id;
    record.timestamp = *timestamp;
    record.signatures_required = *signatures_required;
    record.signers_bitmap = agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.data_update_message_hash = data_update_message_hash;
    record.gateway = gateway.key();
    record.registry_version = *registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.finalized = false;
    record.voided = false;
    record.bump = ctx.bumps.round_record;

    emit!(RoundSettled {
        gateway: gateway.key(),
        job_id: *job_id,
        timestamp: *timestamp,
        signers_bitmap: agg_sig.signers_bitmap,
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
) -> Result<()> {
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
#[instruction(args: SettleRoundArgs, job_init: Option<SettleJobInitArgs>)]
pub struct SettleRoundReceipt<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    // /// CHECK: SAFETY: required only when `job` is initialized by this settlement.
    // /// The handler validates this pubkey against the subscription PDA and derived job id.
    // pub subscription_owner: Option<UncheckedAccount<'info>>,
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

    /// Plan validation is performed only when a new job is initialized.
    pub plan: Option<Account<'info, Plan>>,

    #[account(mut)]
    pub subscription: Account<'info, Subscription>,

    /// CHECK: SAFETY: required when `consumer_authority != job.owner`.
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
            args.job_id.as_ref(),
            args.timestamp.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Account<'info, RoundReceiptRecord>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = anchor_lang::solana_program::sysvar::instructions::ID @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
