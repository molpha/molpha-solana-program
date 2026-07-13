//! `settle_agent_round` — agent pay-per-request settlement (agent-access spec v1).
//!
//! The agent path in front of the existing optimistic rail: the agent's hot `authority`
//! key signed an `AgentRequestAuth` off-chain, the gateway served with **no on-chain
//! preflight**, and this instruction records authoritative usage after the fact, batched.
//!
//! Funding is rail-selected: `UsdcEscrow` debits the agent's prepaid `AgentEscrow.balance`;
//! `X402` debits the gateway's pre-funded `settlement_balance` (the agent already paid the
//! gateway off-chain; `payment_ref` is recorded for audit). Reward crediting, finalize, and
//! dispute are shared with the subscription path — `FreshRound` records are dispute-exposed
//! identically;
//!
//! Unlike `settle_round_receipt`, the transaction signer here **is** the gateway authority:
//! that is the gateway's consent to dispute exposure and to `settlement_balance` debits.

use crate::{
    error::MolphaError,
    events::AgentRoundSettled,
    state::{
        hash_agent_request_auth_fields, hash_agg_sig, AgentEscrow, AgentStatus, AggregatedSig,
        Gateway, Node, PaymentRail, ProtocolConfig, RegistryState, RoundReceiptRecord,
        RECORD_KIND_AGENT_FRESH,
    },
    utils::{
        bitmap::{bitmap_load, for_each_set_bit_u256},
        ed25519::verify_ed25519_signature,
    },
};
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleAgentRoundArgs {
    /// `UsdcEscrow`: must equal the supplied `agent_escrow` key. `X402`: gateway-assigned
    /// payer identity (audit + record-PDA namespace only; no on-chain account required).
    pub agent: Pubkey,
    pub request_nonce: u64,
    pub rail: PaymentRail,
    /// Price ceiling from the agent's signed `AgentRequestAuth`.
    pub max_price: u64,
    /// External-rail settlement reference; `[0; 32]` for `UsdcEscrow`, required for `X402`.
    pub payment_ref: [u8; 32],
    pub feed_id: [u8; 32],
    /// Canonical round timestamp
    pub timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: AggregatedSig,
    /// Feed payload value signed by the aggregate Schnorr coalition.
    pub value: [u8; 32],
}

/// Price from request-time parameters only (never the post-hoc bitmap popcount), so
/// external rails requiring a locked price (x402 `exact`) stay applicable.
pub fn compute_agent_price(config: &ProtocolConfig, signatures_required: u8) -> Result<u64> {
    config
        .agent_round_base
        .checked_add(
            (signatures_required as u64)
                .checked_mul(config.agent_round_per_sig)
                .ok_or(MolphaError::MaxPriceExceeded)?,
        )
        .ok_or_else(|| error!(MolphaError::MaxPriceExceeded))
}

pub fn settle_agent_round<'info>(
    ctx: Context<'_, '_, 'info, 'info, SettleAgentRound<'info>>,
    args: SettleAgentRoundArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let registry = &ctx.accounts.registry_state;

    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let now_ts = clock.unix_timestamp;

    // 1. Gateway consent + bond — enforced by the account constraints (the tx signer is
    //    the gateway authority).

    // 2. Registry — current-version-only for FreshRound settlement.
    require!(
        args.registry_version == registry.current_version,
        MolphaError::InvalidRegistryVersion
    );

    // 3-6. Rail shape, entitlement, replay, agent authorization.
    match args.rail {
        PaymentRail::UsdcEscrow => {
            let escrow = ctx
                .accounts
                .agent_escrow
                .as_ref()
                .ok_or(MolphaError::AgentEscrowRequired)?;
            require_keys_eq!(args.agent, escrow.key(), MolphaError::AgentAccountMismatch);
            require!(
                args.payment_ref == [0u8; 32],
                MolphaError::UnexpectedAgentEscrow
            );

            // 4. Entitlement.
            require!(
                escrow.status == AgentStatus::Active,
                MolphaError::AgentNotActive
            );
            require!(!escrow.is_expired(now_ts), MolphaError::AgentExpired);

            // 5. Replay — strictly increasing nonce (gaps allowed; the gateway orders the
            //    batch). The record PDA `init` on `(agent, request_nonce)` dedups
            //    unconditionally for both rails.
            require!(
                args.request_nonce > escrow.last_request_nonce,
                MolphaError::AgentNonceNotIncreasing
            );

            // 6. Agent authorization — the hot key signed this exact request off-chain.
            let auth_hash = hash_agent_request_auth_fields(
                &args.agent,
                args.request_nonce,
                &args.feed_id,
                args.max_price,
            );
            require!(
                verify_ed25519_signature(
                    &ctx.accounts.instructions_sysvar,
                    &escrow.authority.to_bytes(),
                    &auth_hash,
                )?,
                MolphaError::AgentSigInvalid
            );
        }
        PaymentRail::X402 => {
            // The x402 payment (bound by `payment_ref`) is the authorization; the gateway
            // spends only its own settlement balance, so no on-chain agent identity or
            // signature is required.
            require!(
                ctx.accounts.agent_escrow.is_none(),
                MolphaError::UnexpectedAgentEscrow
            );
            require!(
                args.payment_ref != [0u8; 32],
                MolphaError::MissingPaymentRef
            );
        }
    }

    // 7. Kind shape — quorum bounds on the embedded bitmap (FreshRound only).
    let credited = bitmap_load(&args.agg_sig.signers_bitmap);
    let count = credited.count_ones();
    require!(
        count >= u32::from(args.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    // Cheap inflation bound: an honest selection never exceeds
    // `signatures_required + redundancy_buffer`, so this blocks blatantly padded
    // bitmaps at settlement instead of waiting for a dispute.
    require!(
        count <= u32::from(args.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );
    require!(
        ctx.remaining_accounts.len() == count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    // 8. Price — computed on-chain from protocol config + request-time parameters; bounded
    //    by the agent-signed ceiling and (UsdcEscrow) the owner's max_signers / epoch_cap.
    let price = compute_agent_price(config, args.signatures_required)?;
    require!(price <= args.max_price, MolphaError::MaxPriceExceeded);

    // 9. Funding.
    match args.rail {
        PaymentRail::UsdcEscrow => {
            let escrow = ctx
                .accounts
                .agent_escrow
                .as_mut()
                .ok_or(MolphaError::AgentEscrowRequired)?;
            require!(
                args.signatures_required <= escrow.max_signers,
                MolphaError::SignaturesExceedAgentMax
            );
            escrow.roll_epoch(now_ts, config.agent_epoch_len_secs);
            let epoch_spent = escrow
                .epoch_spent
                .checked_add(price)
                .ok_or(MolphaError::EpochCapExceeded)?;
            require!(epoch_spent <= escrow.epoch_cap, MolphaError::EpochCapExceeded);
            require!(
                escrow.balance >= price,
                MolphaError::InsufficientAgentCredits
            );
            escrow.balance -= price;
            escrow.epoch_spent = epoch_spent;
            escrow.spent_total = escrow
                .spent_total
                .checked_add(price)
                .ok_or(MolphaError::EpochCapExceeded)?;
            escrow.last_request_nonce = args.request_nonce;
        }
        PaymentRail::X402 => {
            let gateway = &mut ctx.accounts.gateway;
            require!(
                gateway.settlement_balance >= price,
                MolphaError::GatewaySettlementUnderfunded
            );
            gateway.settlement_balance -= price;
        }
    }

    // 10. Credit + commitment hashes.
    let reward_per_signature;
    let agg_sig_hash;
    let data_update_message_hash;

    reward_per_signature = config.reward_per_signature;
    let program_id = ctx.program_id;

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
        // Settlement is current-version-only, so the bit position is the slot
        // index directly.
        require!(node.index == bit_pos as u32, MolphaError::InvalidNodeIndex);

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
        .checked_add(config.gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;

    agg_sig_hash = hash_agg_sig(&args.agg_sig);
    data_update_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::DataUpdate {
            feed_id: args.feed_id,
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

    // 11. Record. The price surplus over node + gateway credits is protocol revenue,
    //     implicit in the treasury (identical to subscription revenue).
    let record = &mut ctx.accounts.round_record;
    record.feed_id = args.feed_id;
    record.timestamp = args.timestamp;
    record.signatures_required = args.signatures_required;
    record.signers_bitmap = args.agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.data_update_message_hash = data_update_message_hash;
    record.gateway = ctx.accounts.gateway.key();
    record.registry_version = args.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = config.gateway_fee;
    record.recorded_slot = now_slot;
    record.finalized = false;
    record.voided = false;
    record.kind = RECORD_KIND_AGENT_FRESH;
    record.agent = args.agent;
    record.request_nonce = args.request_nonce;
    record.rail = args.rail as u8;
    record.price = price;
    record.payment_ref = args.payment_ref;
    record.bump = ctx.bumps.round_record;

    // 12. Event.
    emit!(AgentRoundSettled {
        agent: args.agent,
        request_nonce: args.request_nonce,
        rail: args.rail as u8,
        price,
        payment_ref: args.payment_ref,
        feed_id: args.feed_id,
        timestamp: args.timestamp,
        gateway: ctx.accounts.gateway.key(),
        signers_bitmap: record.signers_bitmap,
        credited_count: count,
        recorded_slot: now_slot,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SettleAgentRoundArgs)]
pub struct SettleAgentRound<'info> {
    /// The gateway's registered authority — its signature is the gateway's consent to
    /// dispute exposure (FreshRound) and to settlement-balance debits (X402).
    #[account(mut)]
    pub gateway_authority: Signer<'info>,

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
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump = gateway.bump,
        constraint = gateway.authority == gateway_authority.key() @ MolphaError::UnauthorisedGateway,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Account<'info, Gateway>,

    /// Required iff `args.rail == UsdcEscrow`.
    #[account(mut)]
    pub agent_escrow: Option<Account<'info, AgentEscrow>>,

    #[account(
        init,
        payer = gateway_authority,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::AGENT_SEED_PREFIX,
            args.agent.as_ref(),
            args.request_nonce.to_le_bytes().as_ref(),
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
