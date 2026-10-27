use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::{OffenceProven, RegistryUpdated, RegistryVersionReason},
    state::{
        bps_of, EvidenceAccounts, Offence, ProtocolConfig, PunishmentCase, Registry, RegistryState,
        RegistryTransition,
    },
    utils::slash::{slash_guilty_nodes, SlashInput},
};

pub fn slash_offence<'info>(
    ctx: Context<'info, SlashOffence<'info>>,
    offence: Offence,
) -> Result<()> {
    // Evidence against a superseded version has to be filed within one settlement window of
    // that version's grace ending. A key a node rotated away from, perhaps because it leaked,
    // then stops being able to produce evidence against the node: its versions all retire.
    require!(
        evidence_filing_open(
            ctx.accounts.evidence_registry.load()?.grace_active_until,
            ctx.accounts.protocol_config.max_settlement_delay_seconds,
            Clock::get()?.unix_timestamp,
        ),
        MolphaError::EvidenceExpired
    );

    let guilty_bitmap = offence.validate(&EvidenceAccounts {
        remaining: ctx.remaining_accounts,
        evidence_registry: &ctx.accounts.evidence_registry,
        protocol_config: &ctx.accounts.protocol_config,
        current_registry_version: ctx.accounts.registry_state.current_version,
    })?;
    let reporter = ctx.accounts.reporter.key();

    let clock = Clock::get()?;
    let config = &mut ctx.accounts.protocol_config;

    let batch = slash_guilty_nodes(
        &SlashInput {
            guilty_accounts: ctx.remaining_accounts,
            evidence_registry: &ctx.accounts.evidence_registry,
            guilty_bitmap,
            offence: &offence,
        },
        config.withdrawal_cooldown_slots,
    )?;

    require!(!batch.victims.is_empty(), MolphaError::NothingToPunish);

    let slashed_total = batch.slashed_total();
    let grace_active_until = config.get_grace_active_until();
    // Read policy from the current snapshot before the transition bumps
    // `registry_state.next_version`, which would break exit validation on
    // `new_registry`'s version seed if we loaded it afterwards.
    let redundancy_buffer = ctx.accounts.current_registry.load()?.redundancy_buffer;
    let node_count = ctx.accounts.registry_state.remove_nodes(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            payer: &ctx.accounts.reporter,
            system_program: &ctx.accounts.system_program,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        &batch.victims,
    )?;

    emit!(RegistryUpdated {
        new_version: ctx.accounts.registry_state.current_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::NodePunished,
        updated_at: clock.unix_timestamp,
    });

    // Cancelled reward credits stop being a claim on the treasury.
    config.reward_liability = config
        .reward_liability
        .checked_sub(batch.slashed_rewards)
        .ok_or(error!(MolphaError::RewardLiabilityUnderflow))?;

    // Sweep the forfeited principal out of the deposit vault in one transfer, whatever the size
    // of the guilty set. The amount comes directly from the guilty nodes' locked balances.
    if batch.slashed_locked > 0 {
        require!(
            ctx.accounts.node_deposit_vault.amount >= batch.slashed_locked,
            MolphaError::DepositVaultUnderfunded
        );

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.node_deposit_vault.to_account_info(),
                    to: ctx.accounts.treasury.to_account_info(),
                    authority: ctx.accounts.registry_state.to_account_info(),
                },
                &[&ctx.accounts.registry_state.seeds()],
            ),
            batch.slashed_locked,
        )?;
    }

    // The bounty is funded by what this punishment just recovered: the principal swept in above,
    // plus the reward credits released from `reward_liability`. `challenger_bounty_bps` is capped
    // at 100% of their sum, so it can never be paid out of another party's money.
    let bounty = bps_of(slashed_total, config.challenger_bounty_bps);
    if bounty > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.treasury.to_account_info(),
                    to: ctx.accounts.reporter_usdc.to_account_info(),
                    authority: config.to_account_info(),
                },
                &[&config.seeds()],
            ),
            bounty,
        )?;
    }

    // What the punishment recovered, net of the bounty, belongs to the protocol. Tracking it keeps
    // `sweep_treasury_revenue` from recognizing it as income.
    config.protocol_reserved = config
        .protocol_reserved
        .checked_add(slashed_total - bounty)
        .ok_or(MolphaError::RewardOverflow)?;
    ctx.accounts.treasury.reload()?;
    require!(
        ctx.accounts.treasury.amount >= config.committed()?,
        MolphaError::TreasuryInsolvent
    );

    ctx.accounts.punishment_case.init(
        &offence,
        reporter,
        guilty_bitmap.to_bytes(),
        guilty_bitmap.popcount() as u16,
        slashed_total,
        bounty,
        clock.slot,
        ctx.bumps.punishment_case,
    );

    emit!(OffenceProven {
        offence: offence.kind(),
        guilty_bitmap: guilty_bitmap.to_bytes(),
        evidence_hash: offence.evidence_hash(),
        reporter,
        slashed_total,
        bounty_paid: bounty,
        proven_at: clock.unix_timestamp,
    });
    Ok(())
}

/// Whether evidence against a registry version whose grace ends at `grace_active_until` (`0` while
/// it is current) can still be filed at `now`: until `max_settlement_delay_seconds` after that.
pub fn evidence_filing_open(
    grace_active_until: i64,
    max_settlement_delay_seconds: u64,
    now: i64,
) -> bool {
    let window = i64::try_from(max_settlement_delay_seconds).unwrap_or(i64::MAX);
    grace_active_until == 0 || now < grace_active_until.saturating_add(window)
}

#[derive(Accounts)]
#[instruction(offence: Offence)]
pub struct SlashOffence<'info> {
    #[account(mut)]
    pub reporter: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, offence.registry_version().to_le_bytes().as_ref()],
        bump,
    )]
    pub evidence_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    /// CHECK: the next registry version's PDA. The handler creates it, or takes over one that
    /// `extend_registry` allocated, and writes the snapshot.
    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: UncheckedAccount<'info>,

    #[account(
        init,
        payer = reporter,
        space = PunishmentCase::SPACE,
        seeds = [PunishmentCase::SEED_PREFIX, offence.evidence_hash().as_ref()],
        bump,
    )]
    pub punishment_case: Box<Account<'info, PunishmentCase>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    /// Must already exist. Creating it here would cost two more transaction slots, which the
    /// signer list needs more than a reporter needs the convenience.
    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = reporter,
        associated_token::token_program = token_program,
    )]
    pub reporter_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
