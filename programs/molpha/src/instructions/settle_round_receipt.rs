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
    events::{JobCreated, RoundSettled},
    state::{
        hash_agg_sig, hash_gateway_receipt, hash_request_auth_fields, AggregatedSig, Delegate,
        Gateway, GatewayReceipt, Job, JobInitParams, Node, Plan, ProtocolConfig, RegistryIndex,
        RegistryState, RoundReceiptRecord, Subscription,
    },
    utils::registry::derive_job_id,
    utils::{
        bitmap::{bitmap_is_subset_u256, bitmap_load, for_each_set_bit_u256},
        ed25519::verify_ed25519_signature,
    },
};
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub job_id: [u8; 32],
    pub timestamp: u64,
    pub agg_sig: AggregatedSig,
    pub gateway_receipt: GatewayReceipt,
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
    mut ctx: Context<'_, '_, 'info, 'info, SettleRoundReceipt<'info>>,
    args: SettleRoundArgs,
    job_init: Option<SettleJobInitArgs>,
) -> Result<()> {
    let SettleRoundArgs {
        job_id,
        timestamp,
        agg_sig,
        gateway_receipt,
        value,
        consumer_authority,
    } = &args;

    let clock = Clock::get()?;
    let now_slot = clock.slot;

    let job_is_new = ctx.accounts.job.owner == Pubkey::default();
    let created_job = if job_is_new {
        Some(init_job_if_needed(
            &mut ctx.accounts,
            &args,
            job_init.as_ref(),
            &clock,
            ctx.program_id,
            ctx.bumps.job,
        )?)
    } else {
        validate_existing_job(&ctx.accounts.job, &args, job_init.as_ref(), ctx.bumps.job)?;
        None
    };

    validate_subscription_account(
        &ctx.accounts.subscription,
        ctx.accounts.job.owner,
        ctx.program_id,
    )?;

    let config = &ctx.accounts.protocol_config;
    let registry = &ctx.accounts.registry_state;
    let job = &ctx.accounts.job;
    let gateway = &ctx.accounts.gateway;
    let job_owner = job.owner;
    let signatures_required = job.signatures_required;

    // 1. Job & subscription quota (subscription.is_active() enforced via account constraint).
    require!(
        ctx.accounts.subscription.is_active() && ctx.accounts.subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    // 2. Registry — current-version-only for settlement.
    require!(
        gateway_receipt.registry_version == registry.current_version,
        MolphaError::InvalidRegistryVersion
    );

    // Job binding — single source of truth for job_id.
    require!(*job_id == job.job_id, MolphaError::AggSigJobMismatch);

    // Replay guard — strictly increasing timestamp per job.
    require!(
        *timestamp > job.last_request_timestamp,
        MolphaError::TimestampNotIncreasing
    );
    // 3. Consumer authorization.
    authorize_consumer(
        &ctx.accounts,
        *consumer_authority,
        job_owner,
        ctx.program_id,
        job_is_new,
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
        credited_count >= u32::from(signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        bitmap_is_subset_u256(credited, bitmap_load(&gateway_receipt.selection_bitmap)),
        MolphaError::SignersNotSubsetOfSelection
    );

    // 5. Gateway — bonded, index matches, Ed25519 signature valid.
    require!(
        gateway.index == gateway_receipt.gateway_index,
        MolphaError::GatewayIndexMismatch
    );
    require!(
        !gateway.slashed && gateway.bond >= config.gateway_bond_min,
        MolphaError::GatewayNotBonded
    );

    let agg_sig_hash = hash_agg_sig(agg_sig);
    let data_update_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::DataUpdate {
            job_id: *job_id,
            signatures_required,
            registry_version: gateway_receipt.registry_version,
            signers_bitmap: agg_sig.signers_bitmap,
            value: *value,
            canonical_timestamp: *timestamp as i64,
            agg_sig_s: agg_sig.s,
            commitment_addr: agg_sig.commitment_addr,
        },
        signatures_required,
    );
    let gateway_receipt_hash = hash_gateway_receipt(
        gateway_receipt,
        &request_auth_hash,
        &agg_sig_hash,
        &data_update_message_hash,
    );
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &gateway.authority.to_bytes(),
            &gateway_receipt_hash,
        )?,
        MolphaError::GatewaySigInvalid
    );

    // ---- Credit (deterministic; fixed gateway fee). ----
    let reward_per_signature = config.reward_per_signature;
    let gateway_fee = config.gateway_fee;
    let program_id = ctx.program_id;

    require!(
        ctx.remaining_accounts.len() == credited_count as usize * 2,
        MolphaError::InvalidCreditNodeAccount
    );

    let mut cursor = 0usize;
    for_each_set_bit_u256(credited, |bit_pos| {
        let registry_index_ai = ctx
            .remaining_accounts
            .get(cursor)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        let node_ai = ctx
            .remaining_accounts
            .get(cursor + 1)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        cursor += 2;

        require!(
            registry_index_ai.owner == program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        let registry_index = {
            let data = registry_index_ai.try_borrow_data()?;
            RegistryIndex::try_deserialize(&mut &data[..])?
        };
        // Settlement is current-version-only, so the bit position is the slot index directly.
        require!(
            registry_index.index == bit_pos as u32,
            MolphaError::InvalidNodeIndex
        );

        let mut node: Account<Node> = Account::try_from(node_ai)?;
        require!(
            node.key() == registry_index.node,
            MolphaError::InvalidCreditNodeAccount
        );
        node.pending_rewards = node
            .pending_rewards
            .checked_add(reward_per_signature)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        node.exit(program_id)?;

        Ok(())
    })?;

    let gateway = &mut ctx.accounts.gateway;
    gateway.pending_rewards = gateway
        .pending_rewards
        .checked_add(gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;

    let job = &mut ctx.accounts.job;
    job.last_request_timestamp = *timestamp;
    ctx.accounts.subscription.consume_round()?;

    // 9 (dedup) is enforced by `init` on the record PDA.
    let record = &mut ctx.accounts.round_record;
    record.job_id = *job_id;
    record.timestamp = *timestamp;
    record.selection_bitmap = gateway_receipt.selection_bitmap;
    record.signatures_required = signatures_required;
    record.credited_bitmap = agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.data_update_message_hash = data_update_message_hash;
    record.gateway_index = gateway_receipt.gateway_index;
    record.gateway = ctx.accounts.gateway.key();
    record.registry_version = gateway_receipt.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.finalized = false;
    record.voided = false;
    record.bump = ctx.bumps.round_record;

    if let Some(created_job) = created_job {
        emit!(JobCreated {
            job_id: *job_id,
            owner: created_job.owner,
            plan_type: created_job.plan_type,
            signatures_required: created_job.signatures_required,
            created_at: created_job.created_at,
        });
    }

    emit!(RoundSettled {
        job_id: *job_id,
        timestamp: *timestamp,
        gateway_index: gateway_receipt.gateway_index,
        credited_bitmap: agg_sig.signers_bitmap,
        credited_count,
        reward_per_signature,
        gateway_fee,
        recorded_slot: now_slot,
    });

    Ok(())
}

struct CreatedJobEventData {
    owner: Pubkey,
    plan_type: u8,
    signatures_required: u8,
    created_at: i64,
}

fn init_job_if_needed(
    accounts: &mut SettleRoundReceipt<'_>,
    args: &SettleRoundArgs,
    job_init: Option<&SettleJobInitArgs>,
    clock: &Clock,
    program_id: &Pubkey,
    job_bump: u8,
) -> Result<CreatedJobEventData> {
    let job_init = job_init.ok_or(MolphaError::DataUpdateConfigMismatch)?;
    let job_owner = accounts
        .job_owner
        .as_ref()
        .ok_or(MolphaError::UnauthorisedJobOwner)?
        .key();
    let plan = accounts.plan.as_ref().ok_or(MolphaError::PlanNotActive)?;

    let expected_job_id = derive_job_id(
        &job_owner,
        &job_init.api_config_hash,
        job_init.signatures_required,
    );
    require!(args.job_id == expected_job_id, MolphaError::InvalidJobId);

    validate_subscription_account(&accounts.subscription, job_owner, program_id)?;

    let (expected_plan, _) = Pubkey::find_program_address(
        &[Plan::SEED_PREFIX, &[accounts.subscription.plan_type as u8]],
        program_id,
    );
    require_keys_eq!(plan.key(), expected_plan, MolphaError::PlanNotActive);
    require!(plan.is_active, MolphaError::PlanNotActive);
    require!(
        accounts.subscription.is_active(),
        MolphaError::SubscriptionNotActive
    );
    require!(
        accounts.subscription.job_count < plan.max_jobs,
        MolphaError::JobLimitReached
    );
    require!(
        job_init.signatures_required <= plan.max_signers,
        MolphaError::SignaturesExceedPlanMax
    );

    accounts.job.init(&JobInitParams {
        job_id: args.job_id,
        owner: job_owner,
        api_config_hash: job_init.api_config_hash,
        decimals: job_init.decimals,
        signatures_required: job_init.signatures_required,
        created_at: clock.unix_timestamp,
        bump: job_bump,
    });

    accounts.subscription.job_count = accounts
        .subscription
        .job_count
        .checked_add(1)
        .ok_or(MolphaError::JobLimitReached)?;

    Ok(CreatedJobEventData {
        owner: job_owner,
        plan_type: plan.plan_type as u8,
        signatures_required: job_init.signatures_required,
        created_at: clock.unix_timestamp,
    })
}

fn validate_existing_job(
    job: &Account<'_, Job>,
    args: &SettleRoundArgs,
    job_init: Option<&SettleJobInitArgs>,
    job_bump: u8,
) -> Result<()> {
    require!(job.job_id == args.job_id, MolphaError::AggSigJobMismatch);
    require!(job.bump == job_bump, MolphaError::AggSigJobMismatch);
    let expected_job_id = derive_job_id(&job.owner, &job.api_config_hash, job.signatures_required);
    require!(job.job_id == expected_job_id, MolphaError::InvalidJobId);

    if let Some(job_init) = job_init {
        require!(
            job.api_config_hash == job_init.api_config_hash,
            MolphaError::DataUpdateConfigMismatch
        );
        require!(
            job.decimals == job_init.decimals,
            MolphaError::DataUpdateConfigMismatch
        );
        require!(
            job.signatures_required == job_init.signatures_required,
            MolphaError::DataUpdateConfigMismatch
        );
    }

    Ok(())
}

fn validate_subscription_account(
    subscription: &Account<'_, Subscription>,
    owner: Pubkey,
    program_id: &Pubkey,
) -> Result<()> {
    let (expected_subscription, _) =
        Pubkey::find_program_address(&[Subscription::SEED_PREFIX, owner.as_ref()], program_id);
    require_keys_eq!(
        subscription.key(),
        expected_subscription,
        MolphaError::UnauthorisedJobOwner
    );
    require_keys_eq!(subscription.owner, owner, MolphaError::UnauthorisedJobOwner);
    Ok(())
}

fn authorize_consumer(
    accounts: &SettleRoundReceipt<'_>,
    consumer_authority: Pubkey,
    job_owner: Pubkey,
    program_id: &Pubkey,
    consume_job_quota: bool,
) -> Result<()> {
    if consumer_authority == job_owner {
        require!(
            accounts.consumer_delegate.is_none(),
            MolphaError::UnexpectedDelegateAccount
        );
        return Ok(());
    }

    let delegate_ai = accounts
        .consumer_delegate
        .as_ref()
        .ok_or(MolphaError::DelegateAccountRequired)?;
    let (expected_pda, _bump) = Pubkey::find_program_address(
        &[
            Delegate::SEED_PREFIX,
            job_owner.as_ref(),
            consumer_authority.as_ref(),
        ],
        program_id,
    );
    require_keys_eq!(
        delegate_ai.key(),
        expected_pda,
        MolphaError::UnauthorisedConsumer
    );

    let mut data = delegate_ai.try_borrow_mut_data()?;
    let mut delegate = Delegate::try_deserialize(&mut &data[..])?;
    require_keys_eq!(delegate.owner, job_owner, MolphaError::UnauthorisedConsumer);
    require_keys_eq!(
        delegate.delegate,
        consumer_authority,
        MolphaError::UnauthorisedConsumer
    );
    if consume_job_quota {
        delegate.try_consume_job()?;
    }
    delegate.try_consume_data_request()?;
    delegate.try_serialize(&mut &mut data[..])?;

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SettleRoundArgs, job_init: Option<SettleJobInitArgs>)]
pub struct SettleRoundReceipt<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: SAFETY: required only when `job` is initialized by this settlement.
    /// The handler validates this pubkey against the subscription PDA and derived job id.
    pub job_owner: Option<UncheckedAccount<'info>>,

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

    #[account(
        init_if_needed,
        payer = payer,
        space = Job::SPACE,
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump
    )]
    pub job: Account<'info, Job>,

    #[account(mut)]
    pub subscription: Account<'info, Subscription>,

    /// CHECK: SAFETY: required when `consumer_authority != job.owner`.
    /// The handler checks the delegate PDA, owner/delegate fields, and discriminator before use.
    #[account(mut)]
    pub consumer_delegate: Option<UncheckedAccount<'info>>,

    #[account(mut)]
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
