//! `settle_agent_round` — agent pay-per-request settlement (agent-access spec v1).
//!
//! The agent path in front of the existing optimistic rail: the gateway served with **no
//! on-chain preflight**, and this instruction records authoritative usage after the fact,
//! batched. Every escrow holds **real USDC in its own ATA**; settlement moves the round
//! `price` from that ATA into the protocol treasury (the escrow PDA signs), then credits
//! nodes + gateway exactly as the subscription path does.
//!
//! One `AgentEscrow` shape for both provisioning modes, keyed `["molpha_agent", owner,
//! authority]` and distinguished by whether `owner == authority`:
//!   * **Owner-provisioned** (`owner != authority`): created by `create_agent_escrow`. The hot
//!     `authority` key signed an `AgentRequestAuth` off-chain; on-chain caps (`max_signers`,
//!     `epoch_cap`, `expires_at`, status) and the strictly-increasing nonce are enforced.
//!   * **self-funded** (`owner == authority == payer`): **lazily created here** (the payer
//!     never signs a Molpha tx). Caps are 0; the client's payment into the escrow ATA is
//!     the authorization — no hot key, no caps — and replay is guarded by the round-receipt PDA
//!     `init`. Only escrows with `owner == authority` may be lazily created, so the gateway can
//!     never conjure a privileged owner-provisioned escrow.

use crate::{
    error::MolphaError,
    events::{AgentEscrowCreated, AgentRoundSettled},
    state::{
        hash_agent_request_auth_fields, hash_agg_sig, AgentEscrow, AgentStatus, AggregatedSig,
        Gateway, Node, ProtocolConfig, RegistryState, RoundReceiptRecord,
    },
    utils::{
        bitmap::{bitmap_load, for_each_set_bit_u256},
        ed25519::verify_ed25519_signature,
    },
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleAgentRoundArgs {
    /// Escrow `owner` — a seed of `agent_escrow`. For self-funded escrows this equals `authority`
    /// (both are the payer); the pair drives the `init_if_needed` derivation.
    pub owner: Pubkey,
    /// Escrow `authority` — the other `agent_escrow` seed.
    pub authority: Pubkey,
    pub request_nonce: u64,
    /// Price ceiling from the agent's signed `AgentRequestAuth` (owner-provisioned mode).
    pub max_price: u64,
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
/// external rails requiring a locked price (self-funded `exact`) stay applicable.
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
    let program_id = ctx.program_id;

    let clock = Clock::get()?;
    let now_slot = clock.slot;
    let now_ts = clock.unix_timestamp;

    let epoch_len = config.agent_epoch_len_secs;

    // Registry — current-version-only for FreshRound settlement.
    require!(
        args.registry_version == registry.current_version,
        MolphaError::InvalidRegistryVersion
    );

    let escrow = &mut ctx.accounts.agent_escrow;
    // Lazy self-funded provisioning: if escrow account is not initialized, it means it is a self-funded escrow.
    if escrow.owner == Pubkey::default() {
        require!(
            args.owner == args.authority && args.owner != Pubkey::default(),
            MolphaError::InvalidAgentCaps
        );
        escrow.init_self_funded(args.owner, args.authority, ctx.bumps.agent_escrow);

        emit!(AgentEscrowCreated {
            agent: escrow.key(),
            owner: escrow.owner,
            authority: escrow.authority,
            max_signers: 0,
            epoch_cap: 0,
            expires_at: 0,
        });
    }

    require!(
        escrow.status == AgentStatus::Active,
        MolphaError::AgentNotActive
    );

    let price = compute_agent_price(config, args.signatures_required)?;
    require!(price <= args.max_price, MolphaError::MaxPriceExceeded);

    // Replay + authorization — owner-provisioned only. self-funded relies on the record PDA
    // `init` for replay and on the client's on-chain payment as authorization.
    if escrow.is_owner_provisioned() {
        require!(
            args.request_nonce > escrow.last_request_nonce,
            MolphaError::AgentNonceNotIncreasing
        );

        let auth_hash = hash_agent_request_auth_fields(
            &escrow.key(),
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
        require!(
            args.signatures_required <= escrow.max_signers,
            MolphaError::SignaturesExceedAgentMax
        );
        escrow.roll_epoch(now_ts, epoch_len);
        let epoch_spent = escrow
            .epoch_spent
            .checked_add(price)
            .ok_or(MolphaError::EpochCapExceeded)?;
        require!(
            epoch_spent <= escrow.epoch_cap,
            MolphaError::EpochCapExceeded
        );
        escrow.epoch_spent = epoch_spent;
        escrow.last_request_nonce = args.request_nonce;
    }

    // Quorum bounds on the embedded bitmap (FreshRound only).
    let credited = bitmap_load(&args.agg_sig.signers_bitmap);
    let count = credited.count_ones();
    require!(
        count >= u32::from(args.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        count <= u32::from(args.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );
    require!(
        ctx.remaining_accounts.len() == count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    escrow.spent_total = escrow
        .spent_total
        .checked_add(price)
        .ok_or(MolphaError::EpochCapExceeded)?;

    // 10. Funding — move `price` from the escrow's ATA into the treasury; the escrow PDA signs
    //     with `["molpha_agent", owner, authority, bump]`.
    require!(
        ctx.accounts.escrow_ata.amount >= price,
        MolphaError::InsufficientAgentCredits
    );
    let (owner_key, authority_key, escrow_bump) = { (escrow.owner, escrow.authority, escrow.bump) };
    let bump = [escrow_bump];
    let signer_seeds: &[&[u8]] = &[
        AgentEscrow::SEED_PREFIX,
        owner_key.as_ref(),
        authority_key.as_ref(),
        &bump,
    ];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_ata.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: escrow.to_account_info(),
            },
            &[signer_seeds],
        ),
        price,
    )?;

    // 11. Credit + commitment hashes.
    let reward_per_signature;
    let agg_sig_hash;
    let data_update_message_hash;

    reward_per_signature = config.reward_per_signature;

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

    // 12. Record. The price surplus over node + gateway credits is protocol revenue,
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
    record.price = price;
    record.bump = ctx.bumps.round_record;

    // 13. Event.
    emit!(AgentRoundSettled {
        agent: escrow.key(),
        request_nonce: args.request_nonce,
        price,
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
    /// dispute exposure (FreshRound).
    #[account(mut)]
    pub gateway_authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        mut,
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump = gateway.bump,
        constraint = gateway.authority == gateway_authority.key() @ MolphaError::UnauthorisedGateway,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    /// The escrow, keyed `["molpha_agent", owner, authority]`. `init_if_needed`: an
    /// owner-provisioned escrow already exists (created by `create_agent_escrow`); an
    /// self-funded escrow (`args.owner == args.authority`) is lazily created here on first settle
    /// (the handler enforces that only `owner == authority` escrows may be freshly created).
    /// It is also the authority over `escrow_ata` and signs the transfer to treasury.
    #[account(
        init_if_needed,
        payer = gateway_authority,
        space = AgentEscrow::SPACE,
        seeds = [
            AgentEscrow::SEED_PREFIX,
            args.owner.as_ref(),
            args.authority.as_ref(),
        ],
        bump
    )]
    pub agent_escrow: Box<Account<'info, AgentEscrow>>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidTreasury
    )]
    pub usdc_mint: Box<Account<'info, token::Mint>>,

    /// The escrow's USDC ATA — source of the settled `price`. Pinned to `agent_escrow`. Not
    /// created here: owner escrows create it in `create_agent_escrow`; self-funded clients create +
    /// fund it off-chain before settlement.
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = agent_escrow,
    )]
    pub escrow_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,

    #[account(
        init,
        payer = gateway_authority,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::AGENT_SEED_PREFIX,
            agent_escrow.key().as_ref(),
            args.request_nonce.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = anchor_lang::solana_program::sysvar::instructions::ID @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
