use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{Attestation, CoalitionKey};

use crate::{
    error::MolphaError,
    events::FeedUpdated,
    state::{Feed, FeedValueKind, ProtocolConfig, Registry},
    utils::{
        attestation::{require_registry_active_at, verify_attestation_core},
        require_fresh_timestamp,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitAttestationArgs {
    pub attestation: Attestation,
    /// Preimage of `attestation.payload.value` for values longer than 32 bytes. When present the
    /// signed value must be `keccak256(raw_value)` and the feed stores it as
    /// [`FeedValueKind::Hash`].
    pub raw_value: Option<Vec<u8>>,
    /// Affine sum of the signers' secp256k1 keys (`molpha_verifier::coalition_key`, or any
    /// library's point sum over their `Node` accounts). Unsigned and untrusted: the verifier
    /// checks it projectively against its own sum instead of inverting. A wrong key only fails
    /// the instruction.
    pub coalition_key: CoalitionKey,
}

pub fn submit_attestation<'info>(
    ctx: Context<'info, SubmitAttestation<'info>>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let SubmitAttestationArgs {
        attestation,
        raw_value,
        coalition_key,
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

    require_registry_active_at(&*ctx.accounts.registry.load()?, clock.unix_timestamp)?;
    require_fresh_timestamp(
        // `timestamp` is unix milliseconds; the clock bounds are seconds.
        attestation.payload.timestamp_secs(),
        clock.unix_timestamp,
        ctx.accounts.protocol_config.max_settlement_delay_seconds,
    )?;
    verify_attestation_core(
        ctx.remaining_accounts,
        &ctx.accounts.registry,
        ctx.accounts.protocol_config.min_signers,
        &attestation,
        &coalition_key,
    )?;

    let feed = &mut ctx.accounts.feed;
    if feed.source_id == [0u8; 32] {
        feed.init(
            &attestation,
            value_kind,
            ctx.accounts.submitter.key(),
            ctx.bumps.feed,
        )?;
    } else {
        feed.update(&attestation, value_kind)?;
    }

    emit!(FeedUpdated {
        source_id: feed.source_id,
        registry_version: feed.registry_version,
        timestamp: feed.timestamp,
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
        bump = registry.load()?.bump,
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
