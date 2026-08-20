//! `dispute_round` — the only place Schnorr verification runs.
//!
//! A challenger supplies the committed `AggregatedSig` (preimage of `record.agg_sig_hash`) and
//! forces verification against the registry keys selected by the signature's own
//! `signers_bitmap`. Because the credited set *is* that bitmap and the scheme is a plain-sum
//! coalition key, an over-credited (inflated) set cannot produce a valid aggregate (§1.1).
//!
//! - Invalid selection bitmap, or invalid signature over the credited set
//!   (fabricated round / committed garbage) -> void the round and slash `gateway.bond`
//!   from the gateway bond ATA, paying the challenger `challenger_bounty_bps` and sending
//!   the remainder to treasury. Agent-funded rounds (`record.price > 0`) refund `price`
//!   to the escrow ATA; subscription rounds restore `used_rounds`.
//! - Valid selection bitmap and valid signature -> no fraud; dispute fails
//!   (`DisputeSignatureValid`).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::bitmap::bitmap_is_subset;

use crate::{
    error::{map_attestation_err, MolphaError},
    events::RoundDisputed,
    state::{
        hash_agg_sig, resolve_registry_signers, AgentEscrow, AggregatedSig, Gateway, ProtocolConfig,
        Registry, RoundReceiptRecord, RoundRecordStatus, Subscription,
    },
};

pub fn dispute_round<'info>(
    mut ctx: Context<DisputeRound<'info>>,
    agg_sig: AggregatedSig,
) -> Result<()> {
    let (price, selection_mismatch, is_valid_sig) = {
        let record = &ctx.accounts.round_record;

        require!(
            record.status == RoundRecordStatus::Pending,
            MolphaError::RoundNotPending
        );
        // Challenger must supply the exact committed signature.
        require!(
            hash_agg_sig(&agg_sig) == record.agg_sig_hash,
            MolphaError::DisputeAggSigMismatch
        );
        require!(
            agg_sig.signers_bitmap == record.signers_bitmap,
            MolphaError::CreditedBitmapMismatch
        );

        let registry = ctx.accounts.registry.load()?;
        require!(
            registry.version == record.registry_version,
            MolphaError::InvalidRegistryVersion
        );
        let node_count = u32::from(registry.node_count);
        let expected_selection = molpha_verifier::derive_selection_bitmap(
            &record.source_id,
            record.registry_version,
            record.timestamp as u64,
            node_count,
            record.signatures_required,
            registry.redundancy_buffer,
        )
        .map_err(map_attestation_err)?;
        // Fraudulent selection iff the credited set is NOT a subset of the deterministically
        // derived selection. (Previously inverted — `expected ⊆ signers` — which declared
        // fraud on any honest full-participation round where signers == selection, letting a
        // challenger slash an honest gateway.)
        let selection_mismatch = !bitmap_is_subset(&record.signers_bitmap, &expected_selection);

        // Only run coalition reconstruction / Schnorr verification after deterministic
        // selection validation passes.
        let is_valid_sig = if selection_mismatch {
            false
        } else {
            let ordered = resolve_registry_signers(
                ctx.remaining_accounts,
                ctx.program_id,
                &registry,
                &agg_sig.signers_bitmap,
            )?;
            molpha_verifier::verify_aggregate_over_hash(
                &ordered,
                &agg_sig.s,
                &agg_sig.commitment_addr,
                &record.attestation_message_hash,
            )
            .map_err(map_attestation_err)?
        };
        (record.price, selection_mismatch, is_valid_sig)
    };

    // No fraud iff both deterministic selection and aggregate signature are valid.
    require!(
        selection_mismatch || !is_valid_sig,
        MolphaError::DisputeSignatureValid
    );

    // Undo payer-side settlement effects before slashing.
    let (refund_amount, round_restored) = if price > 0 {
        refund_agent_escrow(&ctx, price)?;
        (price, false)
    } else {
        restore_subscription_round(&mut ctx)?;
        (0u64, true)
    };

    // Fraud: void the round and slash the gateway bond from the gateway ATA.
    let config = &ctx.accounts.protocol_config;
    let slashed_bond = ctx.accounts.gateway.bond;
    require!(
        ctx.accounts.gateway_bond.amount >= slashed_bond,
        MolphaError::InsufficientBond
    );
    let bounty = (slashed_bond as u128)
        .saturating_mul(config.challenger_bounty_bps as u128)
        .checked_div(10_000)
        .unwrap_or(0) as u64;

    if bounty > 0 {
        transfer_from_gateway_bond(
            &ctx,
            ctx.accounts.challenger_usdc.to_account_info(),
            bounty,
        )?;
    }
    let remainder = slashed_bond.saturating_sub(bounty);
    if remainder > 0 {
        transfer_from_gateway_bond(&ctx, ctx.accounts.treasury.to_account_info(), remainder)?;
    }

    let gateway = &mut ctx.accounts.gateway;
    gateway.bond = 0;
    gateway.slashed = true;

    let record = &mut ctx.accounts.round_record;
    record.status = RoundRecordStatus::Voided;

    emit!(RoundDisputed {
        source_id: record.source_id,
        timestamp: record.timestamp,
        gateway: record.gateway,
        slashed_bond,
        challenger_bounty: bounty,
        refund_amount,
        round_restored,
    });

    Ok(())
}

fn transfer_from_treasury<'info>(
    ctx: &Context<DisputeRound<'info>>,
    destination: AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    let bump = [ctx.accounts.protocol_config.bump];
    let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.treasury.to_account_info(),
                to: destination,
                authority: ctx.accounts.protocol_config.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;
    Ok(())
}

fn transfer_from_gateway_bond<'info>(
    ctx: &Context<DisputeRound<'info>>,
    destination: AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    let authority = ctx.accounts.gateway.authority;
    let bump = [ctx.accounts.gateway.bump];
    let signer_seeds: &[&[u8]] = &[Gateway::SEED_PREFIX, authority.as_ref(), &bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.gateway_bond.to_account_info(),
                to: destination,
                authority: ctx.accounts.gateway.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;
    Ok(())
}

fn refund_agent_escrow<'info>(
    ctx: &Context<DisputeRound<'info>>,
    price: u64,
) -> Result<()> {
    require!(
        ctx.accounts.subscription.is_none(),
        MolphaError::DisputeRefundAccountsMissing
    );
    let agent_escrow = ctx
        .accounts
        .agent_escrow
        .as_ref()
        .ok_or_else(|| error!(MolphaError::DisputeRefundAccountsMissing))?;
    let escrow_ata = ctx
        .accounts
        .escrow_ata
        .as_ref()
        .ok_or_else(|| error!(MolphaError::DisputeRefundAccountsMissing))?;

    let record = &ctx.accounts.round_record;
    let expected_escrow = Pubkey::create_program_address(
        &[
            AgentEscrow::SEED_PREFIX,
            agent_escrow.authority.as_ref(),
            record.gateway.as_ref(),
            &[agent_escrow.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        agent_escrow.key(),
        expected_escrow,
        MolphaError::DisputeRefundAccountsMissing
    );

    // let expected_record = Pubkey::create_program_address(
    //     &[
    //         RoundReceiptRecord::AGENT_SEED_PREFIX,
    //         agent_escrow.authority.as_ref(),
    //         record.gateway.as_ref(),
    //         record.source_id.as_ref(),
    //         &[record.signatures_required],
    //         &record.timestamp.to_le_bytes(),
    //         &[record.bump],
    //     ],
    //     ctx.program_id,
    // )
    // .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    // require_keys_eq!(
    //     ctx.accounts.round_record.key(),
    //     expected_record,
    //     MolphaError::DisputeRefundAccountsMissing
    // );

    require_keys_eq!(
        escrow_ata.owner,
        agent_escrow.key(),
        MolphaError::DisputeRefundAccountsMissing
    );
    require_keys_eq!(
        escrow_ata.mint,
        ctx.accounts.protocol_config.usdc_mint,
        MolphaError::InvalidWithdrawDestination
    );

    transfer_from_treasury(ctx, escrow_ata.to_account_info(), price)
}

fn restore_subscription_round<'info>(ctx: &mut Context<DisputeRound<'info>>) -> Result<()> {
    require!(
        ctx.accounts.agent_escrow.is_none() && ctx.accounts.escrow_ata.is_none(),
        MolphaError::DisputeRefundAccountsMissing
    );
    let subscription = ctx
        .accounts
        .subscription
        .as_mut()
        .ok_or_else(|| error!(MolphaError::DisputeRefundAccountsMissing))?;
    let expected_sub = Pubkey::create_program_address(
        &[
            Subscription::SEED_PREFIX,
            subscription.owner.as_ref(),
            &[subscription.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        subscription.key(),
        expected_sub,
        MolphaError::DisputeRefundAccountsMissing
    );

    // Round PDA is seeded with subscription.owner, so owner-direct and
    // delegate-settled rounds of the same subscription share one address.
    let owner = subscription.owner;
    let record = &ctx.accounts.round_record;
    let expected_record = Pubkey::create_program_address(
        &[
            RoundReceiptRecord::SEED_PREFIX,
            owner.as_ref(),
            record.source_id.as_ref(),
            &[record.signatures_required],
            &record.timestamp.to_le_bytes(),
            &[record.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        ctx.accounts.round_record.key(),
        expected_record,
        MolphaError::DisputeRefundAccountsMissing
    );

    subscription.restore_round()
}

#[derive(Accounts)]
pub struct DisputeRound<'info> {
    #[account(mut)]
    pub challenger: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, round_record.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    /// Subscription or agent settlement record. No seeds constraint — same rationale as
    /// `finalize_round` (owner + discriminator). PDA is re-derived in-handler against
    /// the refund accounts for the funding path.
    #[account(mut)]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        mut,
        constraint = gateway_bond.mint == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc,
        constraint = gateway_bond.owner == gateway.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub gateway_bond: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = challenger_usdc.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = challenger_usdc.owner == challenger.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub challenger_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,

    /// Agent path (`record.price > 0`): escrow PDA. Must be `None` for subscription rounds.
    pub agent_escrow: Option<Account<'info, AgentEscrow>>,

    /// Agent path: escrow USDC ATA. Must be `None` for subscription rounds.
    #[account(mut)]
    pub escrow_ata: Option<Account<'info, TokenAccount>>,

    /// Subscription path (`record.price == 0`). Must be `None` for agent rounds.
    #[account(mut)]
    pub subscription: Option<Account<'info, Subscription>>,
}
