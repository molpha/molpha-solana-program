use anchor_lang::prelude::*;

use crate::{
    error::{map_data_update_err, MolphaError},
    events::FeedUpdated,
    state::{
        resolve_registry_signers, Feed, FeedInitParams, FeedUpdateParams, Registry
    },
};
use molpha_verifier::DataUpdate;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitDataUpdateArgs {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub value: Vec<u8>,
    pub canonical_timestamp: i64,
    pub signatures_required: u8,

    /// Schnorr signature R component
    pub agg_sig_s: [u8; 32],
    pub commitment_addr: [u8; 20],
    pub signers_bitmap: [u8; 32],
}

impl From<&SubmitDataUpdateArgs> for DataUpdate {
    fn from(args: &SubmitDataUpdateArgs) -> Self {
        Self {
            source_id: args.source_id,
            registry_version: args.registry_version,
            value: args.value.clone(),
            canonical_timestamp: args.canonical_timestamp,
            signatures_required: args.signatures_required,
            agg_sig_s: args.agg_sig_s,
            commitment_addr: args.commitment_addr,
            signers_bitmap: args.signers_bitmap,
        }
    }
}

pub fn submit_data_update(
    ctx: Context<SubmitDataUpdate>,
    args: SubmitDataUpdateArgs,
) -> Result<()> {
    let (stored_value, value_kind) = Feed::encode_stored_value(&args.value)?;
    let clock = Clock::get()?;
    let feed = &mut ctx.accounts.feed;

    if feed.source_id == [0u8; 32] {
        require!(args.source_id != [0u8; 32], MolphaError::InvalidSourceId);
        feed.init(&FeedInitParams {
            source_id: args.source_id,
            value: stored_value.clone(),
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
            value: stored_value.clone(),
            value_kind,
            canonical_timestamp: args.canonical_timestamp,
            registry_version: args.registry_version,
            signers_bitmap: args.signers_bitmap,
            bump: ctx.bumps.feed,
        });
    }

    let registry = ctx.accounts.registry.load()?;
    let ordered = resolve_registry_signers(
        ctx.remaining_accounts,
        ctx.program_id,
        &registry,
        &args.signers_bitmap,
    )?;
    let payload: DataUpdate = (&args).into();
    molpha_verifier::verify_data_update(
        &payload,
        u32::from(registry.node_count),
        registry.redundancy_buffer,
        &ordered,
    )
    .map_err(map_data_update_err)?;

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
#[instruction(args: SubmitDataUpdateArgs)]
pub struct SubmitDataUpdate<'info> {
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

    pub system_program: Program<'info, System>,
}

#[cfg(test)]
mod tests {
    use super::SubmitDataUpdateArgs;

    /// Borsh size for a 32-byte `value` (u32 length prefix + 32 bytes + fixed fields).
    const ARGS_SERIALIZED_LEN_32: usize = 165;

    fn fixture_args() -> SubmitDataUpdateArgs {
        SubmitDataUpdateArgs {
            source_id: [0x11; 32],
            signatures_required: 3,
            registry_version: 7,
            signers_bitmap: [0x33; 32],
            value: vec![0x44; 32],
            canonical_timestamp: 1_700_000_000,
            agg_sig_s: [0x66; 32],
            commitment_addr: [0x77; 20],
        }
    }

    #[test]
    fn submit_data_update_args_roundtrip() {
        let args = fixture_args();
        let bytes = borsh::to_vec(&args).expect("SubmitDataUpdateArgs should serialize");
        let decoded = borsh::from_slice::<SubmitDataUpdateArgs>(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");

        assert_eq!(decoded.source_id, args.source_id);
        assert_eq!(decoded.registry_version, args.registry_version);
        assert_eq!(decoded.signers_bitmap, args.signers_bitmap);
        assert_eq!(decoded.value, args.value);
        assert_eq!(decoded.canonical_timestamp, args.canonical_timestamp);
        assert_eq!(decoded.agg_sig_s, args.agg_sig_s);
        assert_eq!(decoded.commitment_addr, args.commitment_addr);
    }

    #[test]
    fn submit_data_update_args_serialized_len_is_stable() {
        let bytes = borsh::to_vec(&fixture_args()).expect("SubmitDataUpdateArgs should serialize");
        assert_eq!(bytes.len(), ARGS_SERIALIZED_LEN_32);
    }

    #[test]
    fn submit_data_update_args_preserves_negative_timestamp() {
        let mut args = fixture_args();
        args.canonical_timestamp = -123_456_789;
        let bytes = borsh::to_vec(&args).expect("SubmitDataUpdateArgs should serialize");
        let decoded = borsh::from_slice::<SubmitDataUpdateArgs>(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");
        assert_eq!(decoded.canonical_timestamp, -123_456_789);
    }
}
