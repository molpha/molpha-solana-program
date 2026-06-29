use anchor_lang::prelude::{AccountDeserialize, AccountInfo, Pubkey};

use crate::{
    bitmap::{bitmap_load, for_each_set_bit_u256},
    secp256k1_scalar_is_valid_nonzero, verify_aggregate_over_hash, verify_data_update, DataUpdate,
    DataUpdateError, RegistryIndex, RegistryState, RegistryTransitionType, SignerXy, VIRTUAL_INDEX,
};

#[inline(always)]
pub fn expected_registry_index(
    bit_pos: u32,
    registry: &RegistryState,
    apply_remove_remap: bool,
) -> u32 {
    if !apply_remove_remap {
        return bit_pos;
    }
    if bit_pos == registry.removed_old_index {
        VIRTUAL_INDEX
    } else if registry.last_transition_type == RegistryTransitionType::RemoveSwap
        && bit_pos == registry.moved_old_index
    {
        registry.removed_old_index
    } else {
        bit_pos
    }
}

#[inline(always)]
pub fn validate_remove_transition_for_previous(
    registry: &RegistryState,
) -> Result<(), DataUpdateError> {
    if registry.is_remove_transition() {
        Ok(())
    } else {
        Err(DataUpdateError::InvalidTransitionAccount)
    }
}

pub fn resolve_ordered_signers<'info>(
    accounts: &[AccountInfo<'info>],
    registry: &RegistryState,
    registry_version: u32,
    signers_bitmap: &[u8; 32],
    now: i64,
    program_id: &Pubkey,
) -> Result<(u32, Vec<SignerXy>), DataUpdateError> {
    let signers = bitmap_load(signers_bitmap);
    let signer_count = signers.count_ones();

    let is_current = registry_version == registry.current_version;
    let is_previous_live =
        registry_version == registry.previous_version && now <= registry.previous_expires_at;
    if !is_current && !is_previous_live {
        return Err(DataUpdateError::InvalidRegistryVersion);
    }

    let node_count = if is_current {
        registry.current_node_count
    } else {
        registry.previous_node_count
    };

    if accounts.len() != signer_count as usize {
        return Err(DataUpdateError::MissingSignerAccount);
    }

    let apply_remove_remap =
        !is_current && registry.last_transition_type != RegistryTransitionType::Add;
    if apply_remove_remap {
        validate_remove_transition_for_previous(registry)?;
    }

    let mut ordered = Vec::with_capacity(signer_count as usize);
    let mut account_cursor = 0usize;
    for_each_set_bit_u256(signers, |bit_pos| {
        let account_info = accounts
            .get(account_cursor)
            .ok_or(DataUpdateError::MissingSignerAccount)?;
        if account_info.owner != program_id {
            return Err(DataUpdateError::MissingSignerAccount);
        }

        let (index, x, y) = {
            let data = account_info
                .try_borrow_data()
                .map_err(anchor_lang::error::Error::from)?;
            let registry_index = RegistryIndex::try_deserialize(&mut &data[..])?;
            (
                registry_index.index,
                registry_index.secp256k1_pubkey_x,
                registry_index.secp256k1_pubkey_y,
            )
        };
        account_cursor = account_cursor.saturating_add(1);

        let expected_index = expected_registry_index(bit_pos as u32, registry, apply_remove_remap);
        if index != expected_index {
            return Err(DataUpdateError::InvalidNodeIndex);
        }
        ordered.push((x, y));
        Ok(())
    })?;

    if account_cursor != accounts.len() {
        return Err(DataUpdateError::MissingSignerAccount);
    }
    Ok((node_count, ordered))
}

#[allow(clippy::too_many_arguments)]
pub fn verify_data_update_from_accounts<'info>(
    payload: &DataUpdate,
    registry: &RegistryState,
    redundancy_buffer: u8,
    now: i64,
    accounts: &[AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<(), DataUpdateError> {
    let (node_count, ordered) = resolve_ordered_signers(
        accounts,
        registry,
        payload.registry_version,
        &payload.signers_bitmap,
        now,
        program_id,
    )?;
    verify_data_update(payload, node_count, redundancy_buffer, &ordered)
}

#[allow(clippy::too_many_arguments)]
pub fn verify_aggregate_over_hash_from_accounts<'info>(
    registry: &RegistryState,
    registry_version: u32,
    signers_bitmap: &[u8; 32],
    agg_sig_s: &[u8; 32],
    commitment_addr: &[u8; 20],
    message_hash: &[u8; 32],
    now: i64,
    accounts: &[AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<bool, DataUpdateError> {
    if !secp256k1_scalar_is_valid_nonzero(agg_sig_s) {
        return Ok(false);
    }
    let (_, ordered) = resolve_ordered_signers(
        accounts,
        registry,
        registry_version,
        signers_bitmap,
        now,
        program_id,
    )?;
    verify_aggregate_over_hash(&ordered, agg_sig_s, commitment_addr, message_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remove_swap_registry() -> RegistryState {
        RegistryState {
            authority: Pubkey::default(),
            redundancy_buffer: 0,
            current_version: 2,
            previous_version: 1,
            previous_expires_at: 9_999,
            current_node_count: 7,
            previous_node_count: 8,
            last_transition_type: RegistryTransitionType::RemoveSwap,
            removed_old_index: 1,
            removed_node: Pubkey::new_from_array([0xAA; 32]),
            moved_old_index: 3,
            moved_node: Pubkey::new_from_array([0xBB; 32]),
            grace_secs: 0,
            bump: 0,
        }
    }

    #[test]
    fn expected_registry_index_remove_swap_remaps_removed_and_moved() {
        let registry = remove_swap_registry();
        assert_eq!(expected_registry_index(1, &registry, true), VIRTUAL_INDEX);
        assert_eq!(expected_registry_index(3, &registry, true), 1);
        assert_eq!(expected_registry_index(0, &registry, true), 0);
    }

    #[test]
    fn expected_registry_index_remove_tail_remaps_removed_only() {
        let mut registry = remove_swap_registry();
        registry.last_transition_type = RegistryTransitionType::RemoveTail;
        registry.removed_old_index = 2;
        registry.moved_old_index = VIRTUAL_INDEX;
        registry.moved_node = Pubkey::default();
        assert_eq!(expected_registry_index(2, &registry, true), VIRTUAL_INDEX);
        assert_eq!(expected_registry_index(0, &registry, true), 0);
    }

    #[test]
    fn expected_registry_index_without_remove_remap_is_identity() {
        let registry = remove_swap_registry();
        assert_eq!(expected_registry_index(4, &registry, false), 4);
    }
}
