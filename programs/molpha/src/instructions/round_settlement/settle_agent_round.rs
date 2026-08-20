use crate::{
    error::MolphaError,
    events::{AgentEscrowCreated, AgentRoundSettled},
    state::{
        hash_agent_request_auth_fields, hash_agg_sig, resolve_registry_signers, AgentEscrow,
        AgentStatus, AggregatedSig, Feed, Gateway, ProtocolConfig, Registry, RegistryState,
        RoundReceiptRecord,
    },
    utils::ed25519::verify_ed25519_signature,
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::bitmap::bitmap_load;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleAgentRoundArgs {
    pub authority: Pubkey,
    pub amount: u64,
    pub source_id: [u8; 32],
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: AggregatedSig,
    pub value: Vec<u8>,
}

/// Price from request-time parameters only (never the post-hoc bitmap popcount), so
/// external rails requiring a locked price (self-funded `exact`) stay applicable.
pub fn compute_agent_price(config: &ProtocolConfig, signatures_required: u8) -> Result<u64> {
    config
        .agent_round_base
        .checked_add(
            (signatures_required as u64)
                .checked_mul(config.agent_round_per_sig)
                .ok_or(MolphaError::AmountMismatch)?,
        )
        .ok_or_else(|| error!(MolphaError::AmountMismatch))
}

pub fn settle_agent_round<'info>(
    ctx: Context<SettleAgentRound<'info>>,
    args: SettleAgentRoundArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let registry = ctx.accounts.registry.load()?;
    let gateway = &ctx.accounts.gateway;
    let clock = Clock::get()?;
    let now_slot = clock.slot;

    require!(
        args.registry_version == ctx.accounts.registry_state.current_version
            && registry.version == args.registry_version,
        MolphaError::InvalidRegistryVersion
    );

    require!(
        args.value.len() <= Feed::MAX_VALUE_LEN,
        MolphaError::InvalidValueLength
    );

    let escrow = &mut ctx.accounts.agent_escrow;
    // Lazy self-funded provisioning: if escrow account is not initialized, we initialize it.
    if escrow.authority == Pubkey::default() {
        escrow.init(args.authority, ctx.bumps.agent_escrow);

        emit!(AgentEscrowCreated {
            agent: escrow.key(),
            authority: escrow.authority,
            gateway: gateway.key(),
        });
    }

    require!(
        escrow.status == AgentStatus::Active,
        MolphaError::AgentNotActive
    );

    // Payer must have signed AgentRequestAuth over (agent, gateway, source_id, ts, amount).
    let auth_hash = hash_agent_request_auth_fields(
        &escrow.key(),
        &gateway.key(),
        &args.source_id,
        args.signatures_required,
        args.canonical_timestamp,
        args.amount,
    );
    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &escrow.authority.to_bytes(),
            &auth_hash,
        )?,
        MolphaError::AgentSigInvalid
    );

    let price = compute_agent_price(config, args.signatures_required)?;
    require!(price == args.amount, MolphaError::AmountMismatch);

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

    require!(
        ctx.accounts.escrow_ata.amount >= price,
        MolphaError::InsufficientAgentCredits
    );

    let (authority_key, gateway_key, escrow_bump) =
        { (escrow.authority, gateway.key(), escrow.bump) };
    let bump = [escrow_bump];
    let signer_seeds: &[&[u8]] = &[
        AgentEscrow::SEED_PREFIX,
        authority_key.as_ref(),
        gateway_key.as_ref(),
        &bump,
    ];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.escrow_ata.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: escrow.to_account_info(),
            },
            &[signer_seeds],
        ),
        price,
    )?;

    // Credit + commitment hashes.
    let reward_per_signature = config.reward_per_signature;

    resolve_registry_signers(
        ctx.remaining_accounts,
        ctx.program_id,
        &registry,
        &args.agg_sig.signers_bitmap,
    )?;

    let agg_sig_hash = hash_agg_sig(&args.agg_sig);
    let attestation_message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::AttestationPayload {
            source_id: args.source_id,
            signatures_required: args.signatures_required,
            registry_version: args.registry_version,
            value: args.value.try_into().unwrap(),
            canonical_timestamp: args.canonical_timestamp,
        },
        args.agg_sig.signers_bitmap,
    );

    // Record. The price surplus over node + gateway credits is protocol revenue,
    // implicit in the treasury (identical to subscription revenue).
    let record = &mut ctx.accounts.round_record;
    record.source_id = args.source_id;
    record.timestamp = args.canonical_timestamp;
    record.signatures_required = args.signatures_required;
    record.signers_bitmap = args.agg_sig.signers_bitmap;
    record.agg_sig_hash = agg_sig_hash;
    record.attestation_message_hash = attestation_message_hash;
    record.gateway = ctx.accounts.gateway.key();
    record.registry_version = args.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = config.gateway_fee;
    record.recorded_slot = now_slot;
    record.price = price;
    record.bump = ctx.bumps.round_record;

    emit!(AgentRoundSettled {
        agent: escrow.key(),
        price,
        source_id: args.source_id,
        timestamp: args.canonical_timestamp,
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
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump = gateway.bump,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        init_if_needed,
        payer = gateway_authority,
        space = AgentEscrow::SPACE,
        seeds = [
            AgentEscrow::SEED_PREFIX,
            args.authority.as_ref(),
            gateway.key().as_ref(),
        ],
        bump
    )]
    pub agent_escrow: Box<Account<'info, AgentEscrow>>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidTreasury
    )]
    pub usdc_mint: Box<Account<'info, token::Mint>>,

    /// The escrow's USDC ATA — source of the settled `price`. Pinned to `agent_escrow`.
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
            args.authority.as_ref(),
            gateway.key().as_ref(),
            args.source_id.as_ref(),
            args.signatures_required.to_le_bytes().as_ref(),
            args.canonical_timestamp.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = solana_instructions_sysvar::id() @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
