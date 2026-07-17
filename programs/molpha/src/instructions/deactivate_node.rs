use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_program;

use crate::{
    error::MolphaError,
    events::NodeDeactivated,
    state::{
        Node, NodeOperator, NodeStatus, ProtocolConfig, RegistryState, RegistryTransitionType,
        VIRTUAL_INDEX,
    },
};

pub fn deactivate_node(ctx: Context<DeactivateNode>) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let config = &ctx.accounts.protocol_config;

    let registry = &mut ctx.accounts.registry_state;
    require!(
        now > registry.previous_expires_at,
        MolphaError::TransitionStillInGrace
    );

    let removed_operator = &mut ctx.accounts.removed_operator;

    require!(
        removed_operator.status == NodeStatus::Active,
        MolphaError::NodeNotActive
    );
    require!(
        ctx.accounts.authority.key() == removed_operator.owner
            || ctx.accounts.authority.key() == registry.authority,
        MolphaError::UnauthorizedNodeAuthority
    );

    let removed_index = removed_operator.current_index;
    let last_index = registry.current_node_count.saturating_sub(1);
    require!(
        removed_index < registry.current_node_count,
        MolphaError::InvalidNodeIndex
    );

    let removed_node = &mut ctx.accounts.removed_node;
    require!(
        removed_node.index == removed_index && removed_node.owner == removed_operator.owner,
        MolphaError::InvalidNodeAccount
    );

    // The deactivating node's identity *and* reward balances move to the virtual slot, so a
    // grace-window `finalize_round` can still promote its in-flight pending rounds.
    ctx.accounts.virtual_node.init(
        VIRTUAL_INDEX,
        removed_node.owner,
        removed_node.secp256k1_pubkey_x,
        removed_node.secp256k1_pubkey_y,
        ctx.bumps.virtual_node,
    );
    ctx.accounts.virtual_node.pending_rewards = removed_node.pending_rewards;
    ctx.accounts.virtual_node.claimable_rewards = removed_node.claimable_rewards;

    let old_version = registry.current_version;
    let transition_type;
    let mut moved_node = Pubkey::default();
    let mut moved_old_index = VIRTUAL_INDEX;

    if removed_index == last_index {
        // Tail removal: nothing shifts. Rewards already copied to the virtual slot above; close
        // the freed slot account.
        removed_node.close(ctx.accounts.rent_receiver.to_account_info())?;
        transition_type = RegistryTransitionType::RemoveTail;
    } else {
        // Swap removal: the last-slot node moves into the freed slot, carrying its rewards.
        let last_node_info = &ctx.accounts.last_node;
        require!(
            last_node_info.owner == ctx.program_id,
            MolphaError::InvalidNodeAccount
        );
        let expected_last = Pubkey::find_program_address(
            &[Node::SEED_PREFIX, &last_index.to_le_bytes()],
            ctx.program_id,
        )
        .0;
        require_keys_eq!(
            last_node_info.key(),
            expected_last,
            MolphaError::InvalidNodeAccount
        );

        let (moved_owner, moved_x, moved_y, moved_pending, moved_claimable) = {
            let data = last_node_info.try_borrow_data()?;
            let last_node = Node::try_deserialize(&mut &data[..])
                .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
            require!(last_node.index == last_index, MolphaError::InvalidNodeIndex);
            (
                last_node.owner,
                last_node.secp256k1_pubkey_x,
                last_node.secp256k1_pubkey_y,
                last_node.pending_rewards,
                last_node.claimable_rewards,
            )
        };

        moved_old_index = last_index;

        // The supplied moved operator must be the one owning the last-slot node.
        let expected_moved_operator = Pubkey::find_program_address(
            &[NodeOperator::SEED_PREFIX, moved_owner.as_ref()],
            ctx.program_id,
        )
        .0;
        require_keys_eq!(
            ctx.accounts.moved_operator.key(),
            expected_moved_operator,
            MolphaError::MovedNodeMismatch
        );
        moved_node = ctx.accounts.moved_operator.key();

        // Rewrite the freed slot to represent the moved node (identity + rewards). `index` stays
        // `removed_index` — the slot it now occupies.
        removed_node.owner = moved_owner;
        removed_node.secp256k1_pubkey_x = moved_x;
        removed_node.secp256k1_pubkey_y = moved_y;
        removed_node.pending_rewards = moved_pending;
        removed_node.claimable_rewards = moved_claimable;

        close_account_info(
            last_node_info,
            &ctx.accounts.rent_receiver.to_account_info(),
        )?;

        // Point the moved operator at its new slot.
        {
            let mut data = ctx.accounts.moved_operator.try_borrow_mut_data()?;
            let mut operator = NodeOperator::try_deserialize(&mut &data[..])
                .map_err(|_| error!(MolphaError::MovedNodeMismatch))?;
            operator.current_index = removed_index;
            let mut writer: &mut [u8] = &mut data;
            operator.try_serialize(&mut writer)?;
        }
        transition_type = RegistryTransitionType::RemoveSwap;
    }

    removed_operator.current_index = VIRTUAL_INDEX;
    removed_operator.status = NodeStatus::Deactivated;
    removed_operator.deactivated_at = now;
    removed_operator.withdrawable_at = now.saturating_add(config.withdrawal_cooldown_secs);

    registry.previous_version = old_version;
    registry.previous_node_count = registry.current_node_count;
    registry.previous_expires_at = now.saturating_add(registry.grace_secs);
    registry.current_version = old_version.saturating_add(1);
    registry.current_node_count = registry.current_node_count.saturating_sub(1);
    registry.last_transition_type = transition_type;
    registry.removed_old_index = removed_index;
    registry.removed_node = removed_operator.key();
    registry.moved_old_index = moved_old_index;
    registry.moved_node = moved_node;

    emit!(NodeDeactivated {
        owner: removed_operator.owner,
        removed_index,
        moved_node,
        moved_old_index_or_virtual: moved_old_index,
        current_version: registry.current_version,
        deactivated_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct DeactivateNode<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        mut,
        seeds = [NodeOperator::SEED_PREFIX, authority.key().as_ref()],
        bump = removed_operator.bump,
    )]
    pub removed_operator: Account<'info, NodeOperator>,

    #[account(
        mut,
        seeds = [
            Node::SEED_PREFIX,
            removed_operator.current_index.to_le_bytes().as_ref(),
        ],
        bump = removed_node.bump,
    )]
    pub removed_node: Account<'info, Node>,

    #[account(
        init,
        payer = authority,
        space = Node::SPACE,
        seeds = [
            Node::SEED_PREFIX,
            VIRTUAL_INDEX.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub virtual_node: Account<'info, Node>,

    /// CHECK: swap-remove only (the last-slot `Node`); pass system program for tail removal.
    #[account(mut)]
    pub last_node: UncheckedAccount<'info>,

    /// CHECK: swap-remove only (the moved node's `NodeOperator`); pass system program for tail removal.
    #[account(mut)]
    pub moved_operator: UncheckedAccount<'info>,

    /// CHECK: receives reclaimed rent from closed slot accounts.
    #[account(mut)]
    pub rent_receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

fn close_account_info(account: &AccountInfo<'_>, receiver: &AccountInfo<'_>) -> Result<()> {
    let lamports = account.lamports();
    **account.try_borrow_mut_lamports()? = 0;
    **receiver.try_borrow_mut_lamports()? = receiver.lamports().saturating_add(lamports);
    account.assign(&system_program::ID);
    account.resize(0)?;
    Ok(())
}
