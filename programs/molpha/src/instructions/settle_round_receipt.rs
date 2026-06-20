//! `settle_round_receipt` — optimistic happy path (no Schnorr verify).
//!
//! Verifies two cheap Ed25519 signatures (consumer + gateway), stores the committed
//! `AggregatedSig` *without verifying it*, and credits the participation set embedded in that
//! signature (`agg_sig.signers_bitmap`). Schnorr verification runs only on `dispute_round`.
//!
//! The v0.2 spec batches several rounds (`Vec<SettleEntry>`) into one transaction. Because the
//! per-round cost is flat (§8), this is implemented as a single-round instruction; clients pack
//! multiple of these into one transaction up to the 1232-byte limit for identical effect, with
//! far simpler and safer account handling.

use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_agg_sig, hash_gateway_receipt, hash_request_auth_fields, AggregatedSig, Delegate,
        Gateway, GatewayReceipt, Job, Node, ProtocolConfig, RegistryIndex, RegistryState,
        RoundReceiptRecord, Subscription,
    },
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
    /// Consumer Ed25519 key that signed `H(RequestAuth)` (job owner or a registered delegate).
    pub consumer_authority: Pubkey,
}

pub fn settle_round_receipt<'info>(
    ctx: Context<'_, '_, 'info, 'info, SettleRoundReceipt<'info>>,
    args: SettleRoundArgs,
) -> Result<()> {
    let SettleRoundArgs {
        job_id,
        timestamp,
        agg_sig,
        gateway_receipt,
        consumer_authority,
    } = &args;

    let config = &ctx.accounts.protocol_config;
    let registry = &ctx.accounts.registry_state;
    let job = &ctx.accounts.job;
    let gateway = &ctx.accounts.gateway;

    let clock = Clock::get()?;
    let now_slot = clock.slot;

    // 1. Job & subscription quota (subscription.is_active() enforced via account constraint).
    require!(
        job.used_rounds < job.max_rounds,
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
    let is_owner = *consumer_authority == job.owner;
    if is_owner {
        require!(
            ctx.accounts.consumer_delegate.is_none(),
            MolphaError::UnexpectedDelegateAccount
        );
    } else {
        let delegate_ai = ctx
            .accounts
            .consumer_delegate
            .as_ref()
            .ok_or(MolphaError::DelegateAccountRequired)?;
        let (expected_pda, _bump) = Pubkey::find_program_address(
            &[
                Delegate::SEED_PREFIX,
                job.owner.as_ref(),
                consumer_authority.as_ref(),
            ],
            ctx.program_id,
        );
        require_keys_eq!(
            delegate_ai.key(),
            expected_pda,
            MolphaError::UnauthorisedConsumer
        );

        let mut data = delegate_ai.try_borrow_mut_data()?;
        let mut delegate = Delegate::try_deserialize(&mut &data[..])?;
        require_keys_eq!(delegate.owner, job.owner, MolphaError::UnauthorisedConsumer);
        require_keys_eq!(
            delegate.delegate,
            *consumer_authority,
            MolphaError::UnauthorisedConsumer
        );
        delegate.try_consume_data_request()?;
        delegate.try_serialize(&mut &mut data[..])?;
    }
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
        credited_count >= u32::from(job.signatures_required),
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
    let gateway_receipt_hash =
        hash_gateway_receipt(gateway_receipt, &request_auth_hash, &agg_sig_hash);
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
    job.used_rounds = job.used_rounds.saturating_add(1);
    job.last_request_timestamp = *timestamp;

    // 9 (dedup) is enforced by `init` on the record PDA.
    let record = &mut ctx.accounts.round_record;
    record.job_id = *job_id;
    record.timestamp = *timestamp;
    record.selection_bitmap = gateway_receipt.selection_bitmap;
    record.signatures_required = job.signatures_required;
    record.credited_bitmap = agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.gateway_index = gateway_receipt.gateway_index;
    record.gateway = ctx.accounts.gateway.key();
    record.registry_version = gateway_receipt.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.finalized = false;
    record.voided = false;
    record.bump = ctx.bumps.round_record;

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
        mut,
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump = job.bump,
    )]
    pub job: Account<'info, Job>,

    #[account(
        seeds = [Subscription::SEED_PREFIX, job.owner.as_ref()],
        bump = subscription.bump,
        constraint = subscription.is_active() @ MolphaError::SubscriptionInactiveOrExhausted,
    )]
    pub subscription: Account<'info, Subscription>,

    /// Required when `consumer_authority != job.owner`; enforces delegate request quota.
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
