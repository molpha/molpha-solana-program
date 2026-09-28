//! Permissionless: credit finalized epoch shares to node reward balances.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::EpochRewardCredited,
    state::{EpochState, Node, NodeStatus, ProtocolConfig},
};

/// Batched: `slots[i]` is the roster slot of `remaining_accounts[i]` (a writable `Node`). Each
/// weighted slot is credited once, `floor(pool · weight / weight_total)`, into the node's existing
/// `claimable_rewards`. A tombstoned node's share is forfeited: it stays in the epoch residual and
/// rolls into a later pool when the epoch closes.
pub fn credit_epoch_rewards<'info>(
    ctx: Context<'info, CreditEpochRewards<'info>>,
    _epoch: u64,
    slots: Vec<u16>,
) -> Result<()> {
    require!(
        !slots.is_empty() && slots.len() == ctx.remaining_accounts.len(),
        MolphaError::InvalidRosterSlot
    );

    let mut state = ctx.accounts.epoch_state.load_mut()?;
    require!(state.is_finalized(), MolphaError::EpochNotFinalized);
    let config = &mut ctx.accounts.protocol_config;

    for (slot, node_ai) in slots.iter().zip(ctx.remaining_accounts.iter()) {
        let slot = usize::from(*slot);
        require_keys_eq!(
            node_ai.key(),
            state.roster_key(slot)?,
            MolphaError::InvalidNodeAccount
        );
        require!(node_ai.is_writable, MolphaError::InvalidNodeAccount);

        let weight = state.weight(slot);
        require!(
            weight > 0 && !state.is_credited(slot),
            MolphaError::NothingToCredit
        );
        state.mark_credited(slot);
        state.credited_count += 1;

        let mut node: Account<Node> = Account::try_from(node_ai)?;
        let forfeited = node.status == NodeStatus::Tombstoned;
        let amount = if forfeited { 0 } else { state.payout(weight) };
        if amount > 0 {
            node.add_rewards(amount)?;
            node.exit(&crate::ID)?;
            state.paid = state
                .paid
                .checked_add(amount)
                .ok_or(MolphaError::RewardOverflow)?;
            config.pool_liability = config
                .pool_liability
                .checked_sub(amount)
                .ok_or(MolphaError::RewardLiabilityUnderflow)?;
            config.reward_liability = config
                .reward_liability
                .checked_add(amount)
                .ok_or(MolphaError::RewardOverflow)?;
        }

        emit!(EpochRewardCredited {
            epoch: state.epoch,
            node: node_ai.key(),
            weight,
            amount,
            forfeited,
        });
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct CreditEpochRewards<'info> {
    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [EpochState::SEED_PREFIX, epoch.to_le_bytes().as_ref()],
        bump = epoch_state.load()?.bump,
    )]
    pub epoch_state: AccountLoader<'info, EpochState>,
}
