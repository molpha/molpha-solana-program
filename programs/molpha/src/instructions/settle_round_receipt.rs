use crate::{
    error::MolphaError,
    events::RoundSettled,
    state::{
        hash_agg_sig, hash_request_auth_fields, AggregatedSig, Delegate, Feed, Gateway, Node,
        ProtocolConfig, RegistryState, RoundReceiptRecord, Subscription,
    },
    utils::{ed25519::verify_ed25519_signature, registry::derive_feed_id},
};
use anchor_lang::prelude::*;
use molpha_verifier::bitmap::{bitmap_load, for_each_set_bit_u256};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleRoundArgs {
    pub timestamp: u64,
    pub api_config_hash: [u8; 32],
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: AggregatedSig,
    pub value: Vec<u8>,
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
    let registry = &ctx.accounts.registry_state;
    let gateway = &mut ctx.accounts.gateway;

    require!(
        subscription.is_active() && subscription.has_round_quota(),
        MolphaError::SubscriptionInactiveOrExhausted
    );

    let feed_id = derive_feed_id(
        args.consumer_authority,
        args.api_config_hash,
        args.signatures_required,
    );

    require!(
        args.registry_version == registry.current_version,
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

    let request_auth_hash = hash_request_auth_fields(&feed_id, args.timestamp);
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
    let data_update_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::DataUpdate {
            feed_id,
            signatures_required: args.signatures_required,
            registry_version: args.registry_version,
            signers_bitmap: args.agg_sig.signers_bitmap,
            value: args.value,
            canonical_timestamp: args.timestamp as i64,
            agg_sig_s: args.agg_sig.s,
            commitment_addr: args.agg_sig.commitment_addr,
        },
        args.signatures_required,
    );

    // ---- Validate credited nodes; rewards are recorded on the round receipt only. ----
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
        let node: Account<Node> = Account::try_from(node_ai)?;
        // Settlement is current-version-only, so the bit position is the slot index directly.
        require!(node.index == bit_pos as u32, MolphaError::InvalidNodeIndex);

        Ok(())
    })?;

    ctx.accounts.subscription.consume_round()?;

    // 9 (dedup) is enforced by `init` on the record PDA.
    let record = &mut ctx.accounts.round_record;
    record.feed_id = feed_id;
    record.timestamp = args.timestamp;
    record.signatures_required = args.signatures_required;
    record.signers_bitmap = args.agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.data_update_message_hash = data_update_message_hash;
    record.gateway = gateway.key();
    record.registry_version = args.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.price = 0;
    record.bump = ctx.bumps.round_record;

    emit!(RoundSettled {
        gateway: gateway.key(),
        feed_id,
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

    // /// CHECK: SAFETY: required only when `job` is initialized by this settlement.
    // /// The handler validates this pubkey against the subscription PDA and derived feed id.
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
            args.consumer_authority.as_ref(),
            args.api_config_hash.as_ref(),
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
