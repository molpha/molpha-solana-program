use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::FeedUpdated,
    instructions::data_update_core::build_coalition_and_verify,
    state::{Feed, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitDataUpdateArgs {
    pub job_id: [u8; 32],
    pub signatures_required: u8,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub value: [u8; 32],
    pub canonical_timestamp: i64,
    /// Schnorr signature R component
    pub agg_sig_s: [u8; 32],
    pub commitment_addr: [u8; 20],
}

pub fn submit_data_update(
    ctx: Context<SubmitDataUpdate>,
    args: SubmitDataUpdateArgs,
) -> Result<()> {
    let feed = &mut ctx.accounts.feed;
    let registry = &ctx.accounts.registry_state;

    require!(
        args.canonical_timestamp > feed.canonical_timestamp,
        MolphaError::FeedNotNewer
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    build_coalition_and_verify(
        &args,
        registry,
        now,
        ctx.remaining_accounts,
        ctx.program_id,
        args.signatures_required,
        registry.redundancy_buffer,
    )?;

    feed.value = args.value;
    feed.canonical_timestamp = args.canonical_timestamp;
    feed.registry_version = args.registry_version;
    feed.signers_bitmap = args.signers_bitmap;
    feed.signatures_required = args.signatures_required;
    feed.last_updated_slot = clock.slot;

    emit!(FeedUpdated {
        job_id: args.job_id,
        registry_version: args.registry_version,
        canonical_timestamp: args.canonical_timestamp,
        value: args.value,
        signers_bitmap: args.signers_bitmap,
        last_updated_slot: clock.slot,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SubmitDataUpdateArgs)]
pub struct SubmitDataUpdate<'info> {
    pub submitter: Signer<'info>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        mut,
        seeds = [Feed::SEED_PREFIX, args.job_id.as_ref()],
        bump = feed.bump,
        constraint = feed.job_id == args.job_id @ MolphaError::DataUpdateJobMismatch,
    )]
    pub feed: Account<'info, Feed>,
}

#[cfg(test)]
mod tests {
    use super::SubmitDataUpdateArgs;
    use anchor_lang::{AnchorDeserialize, AnchorSerialize};

    const ARGS_SERIALIZED_LEN: usize = 224;

    fn fixture_args() -> SubmitDataUpdateArgs {
        SubmitDataUpdateArgs {
            job_id: [0x11; 32],
            signatures_required: 3,
            registry_version: 7,
            signers_bitmap: [0x33; 32],
            value: [0x44; 32],
            canonical_timestamp: 1_700_000_000,
            agg_sig_s: [0x66; 32],
            commitment_addr: [0x77; 20],
        }
    }

    #[test]
    fn submit_data_update_args_roundtrip() {
        let args = fixture_args();
        let bytes = args
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        let decoded = SubmitDataUpdateArgs::try_from_slice(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");

        assert_eq!(decoded.job_id, args.job_id);
        assert_eq!(decoded.registry_version, args.registry_version);
        assert_eq!(decoded.signers_bitmap, args.signers_bitmap);
        assert_eq!(decoded.value, args.value);
        assert_eq!(decoded.canonical_timestamp, args.canonical_timestamp);
        assert_eq!(decoded.agg_sig_s, args.agg_sig_s);
        assert_eq!(decoded.commitment_addr, args.commitment_addr);
    }

    #[test]
    fn submit_data_update_args_serialized_len_is_stable() {
        let bytes = fixture_args()
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        assert_eq!(bytes.len(), ARGS_SERIALIZED_LEN);
    }

    #[test]
    fn submit_data_update_args_preserves_negative_timestamp() {
        let mut args = fixture_args();
        args.canonical_timestamp = -123_456_789;
        let bytes = args
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        let decoded = SubmitDataUpdateArgs::try_from_slice(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");
        assert_eq!(decoded.canonical_timestamp, -123_456_789);
    }
}
