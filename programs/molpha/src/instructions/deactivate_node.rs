use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_program;

use crate::{
    error::MolphaError,
    events::NodeDeactivated,
    state::{
        Node, NodeStatus, ProtocolConfig, RegistryIndex, RegistryState, RegistryTransitionType, VIRTUAL_INDEX,
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

    let removed_node = &mut ctx.accounts.removed_node;

    require!(
        removed_node.status == NodeStatus::Active,
        MolphaError::NodeNotActive
    );
    require!(
        ctx.accounts.authority.key() == removed_node.owner
            || ctx.accounts.authority.key() == registry.authority,
        MolphaError::UnauthorizedNodeAuthority
    );

    let removed_index = removed_node.current_index;
    let last_index = registry.current_node_count.saturating_sub(1);
    require!(
        removed_index < registry.current_node_count,
        MolphaError::InvalidNodeIndex
    );

    let removed_index_account = &mut ctx.accounts.removed_index_account;
    require!(
        removed_index_account.index == removed_index
            && removed_index_account.node == removed_node.key(),
        MolphaError::InvalidRegistryIndexAccount
    );

    let old_version = registry.current_version;
    let transition_type;
    let mut moved_node = Pubkey::default();
    let mut moved_old_index = VIRTUAL_INDEX;

    let virtual_index = &mut ctx.accounts.virtual_registry_index;
    virtual_index.index = VIRTUAL_INDEX;
    virtual_index.node = removed_index_account.node;
    virtual_index.secp256k1_pubkey_x = removed_index_account.secp256k1_pubkey_x;
    virtual_index.secp256k1_pubkey_y = removed_index_account.secp256k1_pubkey_y;
    virtual_index.bump = ctx.bumps.virtual_registry_index;

    if removed_index == last_index {
        removed_index_account.close(ctx.accounts.rent_receiver.to_account_info())?;
        transition_type = RegistryTransitionType::RemoveTail;
    } else {
        let last_index_info = &ctx.accounts.last_index_account;
        require!(
            last_index_info.owner == ctx.program_id,
            MolphaError::InvalidRegistryIndexAccount
        );
        let last_index_data = last_index_info.try_borrow_data()?;
        let last_index_account = RegistryIndex::try_deserialize(&mut &last_index_data[..])
            .map_err(|_| error!(MolphaError::InvalidRegistryIndexAccount))?;
        let expected_last_index = Pubkey::find_program_address(
            &[RegistryIndex::SEED_PREFIX, &last_index.to_le_bytes()],
            ctx.program_id,
        )
        .0;
        require_keys_eq!(
            last_index_info.key(),
            expected_last_index,
            MolphaError::InvalidRegistryIndexAccount
        );
        require!(
            last_index_account.index == last_index,
            MolphaError::InvalidNodeIndex
        );
        moved_node = last_index_account.node;
        moved_old_index = last_index;

        require!(
            ctx.accounts.moved_node.key() == moved_node,
            MolphaError::MovedNodeMismatch
        );

        removed_index_account.node = moved_node;
        removed_index_account.secp256k1_pubkey_x = last_index_account.secp256k1_pubkey_x;
        removed_index_account.secp256k1_pubkey_y = last_index_account.secp256k1_pubkey_y;

        close_account_info(
            last_index_info,
            &ctx.accounts.rent_receiver.to_account_info(),
        )?;
        ctx.accounts.moved_node.current_index = removed_index;
        transition_type = RegistryTransitionType::RemoveSwap;
    }

    removed_node.current_index = VIRTUAL_INDEX;
    removed_node.status = NodeStatus::Deactivated;
    removed_node.deactivated_at = now;
    removed_node.withdrawable_at = now.saturating_add(config.withdrawal_cooldown_secs);

    registry.previous_version = old_version;
    registry.previous_node_count = registry.current_node_count;
    registry.previous_expires_at = now.saturating_add(registry.grace_secs);
    registry.current_version = old_version.saturating_add(1);
    registry.current_node_count = registry.current_node_count.saturating_sub(1);
    registry.last_transition_type = transition_type;
    registry.removed_old_index = removed_index;
    registry.removed_node = removed_node.key();
    registry.moved_old_index = moved_old_index;
    registry.moved_node = moved_node;

    emit!(NodeDeactivated {
        owner: removed_node.owner,
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
        seeds = [Node::SEED_PREFIX, authority.key().as_ref()],
        bump = removed_node.bump,
    )]
    pub removed_node: Account<'info, Node>,

    #[account(
        mut,
        seeds = [
            RegistryIndex::SEED_PREFIX,
            removed_node.current_index.to_le_bytes().as_ref(),
        ],
        bump = removed_index_account.bump,
    )]
    pub removed_index_account: Account<'info, RegistryIndex>,

    #[account(
        init,
        payer = authority,
        space = RegistryIndex::SPACE,
        seeds = [
            RegistryIndex::SEED_PREFIX,
            VIRTUAL_INDEX.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub virtual_registry_index: Account<'info, RegistryIndex>,

    /// CHECK: required only in swap-remove branch.
    #[account(mut)]
    pub last_index_account: UncheckedAccount<'info>,

    /// CHECK: required only in swap-remove branch.
    #[account(mut)]
    pub moved_node: Account<'info, Node>,

    /// CHECK: receives reclaimed rent from closed index accounts.
    #[account(mut)]
    pub rent_receiver: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

fn close_account_info(account: &AccountInfo<'_>, receiver: &AccountInfo<'_>) -> Result<()> {
    let lamports = account.lamports();
    **account.try_borrow_mut_lamports()? = 0;
    **receiver.try_borrow_mut_lamports()? = receiver.lamports().saturating_add(lamports);
    account.assign(&system_program::ID);
    account.realloc(0, false)?;
    Ok(())
}
