use anchor_lang::prelude::*;
use molpha_verifier::solana::verify_attestation;
use molpha_verifier::Attestation;
use solana_keccak_hasher::hashv;

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    events::FeedUpdated,
    state::{Feed, FeedValueKind, ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitAttestationArgs {
    pub attestation: Attestation,
    /// Preimage of `attestation.payload.value` for values longer than 32 bytes. When present the
    /// signed value must be `keccak256(raw_value)` and the feed stores it as
    /// [`FeedValueKind::Hash`].
    pub raw_value: Option<Vec<u8>>,
}

pub fn submit_attestation(
    ctx: Context<SubmitAttestation>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let SubmitAttestationArgs {
        attestation,
        raw_value,
    } = args;

    let value_kind = match raw_value {
        Some(raw_value) => {
            require!(
                raw_value.len() <= Feed::MAX_VALUE_LEN
                    && hashv(&[raw_value.as_slice()]).to_bytes() == attestation.payload.value,
                MolphaError::InvalidRawValue
            );
            FeedValueKind::Hash
        }
        None => FeedValueKind::Value,
    };

    let clock = Clock::get()?;
    require!(
        ctx.accounts
            .registry
            .load()?
            .is_active(clock.unix_timestamp),
        MolphaError::RegistryNotActive
    );
    require_fresh_canonical_timestamp(
        attestation.payload.canonical_timestamp,
        clock.unix_timestamp,
        ctx.accounts.protocol_config.max_settlement_delay_seconds,
    )?;
    require!(
        attestation.payload.signatures_required >= ctx.accounts.protocol_config.min_signers,
        MolphaError::SignaturesBelowMinSigners
    );

    let feed = &mut ctx.accounts.feed;
    if feed.source_id == [0u8; 32] {
        feed.init(&attestation, value_kind, ctx.accounts.submitter.key(), ctx.bumps.feed)?;
    } else {
        feed.update(&attestation, value_kind)?;
    }

    verify_attestation(
        &attestation,
        &ctx.accounts.registry.to_account_info(),
        ctx.remaining_accounts,
    )
    .map_err(map_attestation_accounts_err)?;

    emit!(FeedUpdated {
        source_id: feed.source_id,
        registry_version: feed.registry_version,
        canonical_timestamp: feed.canonical_timestamp,
        value: feed.value,
        value_kind,
        signers_bitmap: feed.signers_bitmap,
        last_updated_slot: clock.slot,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SubmitAttestationArgs)]
pub struct SubmitAttestation<'info> {
    #[account(mut)]
    pub submitter: Signer<'info>,

    #[account(
        seeds = [Registry::SEED_PREFIX, args.attestation.payload.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        init_if_needed,
        payer = submitter,
        space = Feed::SPACE,
        seeds = [
            Feed::SEED_PREFIX,
            args.attestation.payload.source_id.as_ref(),
            args.attestation.payload.signatures_required.to_le_bytes().as_ref(),
            submitter.key().as_ref()
        ],
        bump
    )]
    pub feed: Account<'info, Feed>,

    #[account(seeds = [ProtocolConfig::SEED_PREFIX], bump = protocol_config.bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    pub system_program: Program<'info, System>,
}
