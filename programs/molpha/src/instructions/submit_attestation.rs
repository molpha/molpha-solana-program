use anchor_lang::prelude::*;

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    events::FeedUpdated,
    state::{Feed, FeedInitParams, FeedUpdateParams, ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};
use molpha_verifier::{
    Attestation, AttestationPayload, SchnorrSignature, solana::verify_attestation,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitAttestationArgs {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub value: [u8; 32],
    pub canonical_timestamp: u64,
    pub signatures_required: u8,

    /// Schnorr signature R component
    pub agg_sig_s: [u8; 32],
    pub commitment: [u8; 20],
    pub signers_bitmap: [u8; 32],
}

impl From<&SubmitAttestationArgs> for Attestation {
    fn from(args: &SubmitAttestationArgs) -> Self {
        Attestation {
            payload: AttestationPayload {
                source_id: args.source_id,
                registry_version: args.registry_version,
                value: args.value,
                canonical_timestamp: args.canonical_timestamp,
                signatures_required: args.signatures_required,
            },
            signature: SchnorrSignature {
                agg_sig_s: args.agg_sig_s,
                commitment: args.commitment,
                signers_bitmap: args.signers_bitmap,
            },
        }
    }
}

pub fn submit_attestation(
    ctx: Context<SubmitAttestation>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let (stored_value, value_kind) = Feed::encode_stored_value(&args.value)?;
    let clock = Clock::get()?;
    require!(
        ctx.accounts
            .registry
            .load()?
            .is_active(clock.unix_timestamp),
        MolphaError::RegistryNotActive
    );
    require_fresh_canonical_timestamp(
        args.canonical_timestamp,
        clock.unix_timestamp,
        ctx.accounts.protocol_config.max_settlement_delay_seconds,
    )?;
    require!(
        args.signatures_required >= ctx.accounts.protocol_config.min_signers,
        MolphaError::SignaturesBelowMinSigners
    );

    let feed = &mut ctx.accounts.feed;

    if feed.source_id == [0u8; 32] {
        require!(args.source_id != [0u8; 32], MolphaError::InvalidSourceId);
        feed.init(&FeedInitParams {
            source_id: args.source_id,
            value: stored_value,
            value_kind,
            canonical_timestamp: args.canonical_timestamp,
            signatures_required: args.signatures_required,
            registry_version: args.registry_version,
            signers_bitmap: args.signers_bitmap,
            bump: ctx.bumps.feed,
        });
    } else {
        require!(
            args.canonical_timestamp > feed.canonical_timestamp,
            MolphaError::FeedNotNewer
        );
        feed.update(&FeedUpdateParams {
            value: stored_value,
            value_kind,
            canonical_timestamp: args.canonical_timestamp,
            registry_version: args.registry_version,
            signers_bitmap: args.signers_bitmap,
            bump: ctx.bumps.feed,
        });
    }

    let attestation: Attestation = (&args).into();
    verify_attestation(
        &attestation,
        &ctx.accounts.registry.to_account_info(),
        ctx.remaining_accounts,
    )
    .map_err(map_attestation_accounts_err)?;

    emit!(FeedUpdated {
        source_id: feed.source_id,
        registry_version: args.registry_version,
        canonical_timestamp: args.canonical_timestamp,
        value: stored_value,
        value_kind,
        signers_bitmap: args.signers_bitmap,
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
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        init_if_needed,
        payer = submitter,
        space = Feed::SPACE,
        seeds = [
            Feed::SEED_PREFIX,
            args.source_id.as_ref(),
            args.signatures_required.to_le_bytes().as_ref(),
            submitter.key().as_ref()
        ],
        bump
    )]
    pub feed: Account<'info, Feed>,

    #[account(seeds = [ProtocolConfig::SEED_PREFIX], bump = protocol_config.bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    pub system_program: Program<'info, System>,
}
