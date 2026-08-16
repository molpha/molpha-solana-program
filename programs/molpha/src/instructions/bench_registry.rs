//! Feature-gated compute-unit probes for immutable registry snapshots.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    state::{resolve_registry_signers, Node, Registry},
};
use molpha_verifier::bitmap::{bitmap_load, for_each_set_bit_u256};

pub const BENCH_FULL_REGISTRY_SEED_PREFIX: &[u8] = b"bench_full_registry";
pub const BENCH_FULL_REGISTRY_MAX_NODES: usize = 256;

#[account(zero_copy)]
#[repr(C)]
pub struct BenchFullRegistry {
    pub version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub bump: u8,
    pub nodes: [[u8; 32]; BENCH_FULL_REGISTRY_MAX_NODES],
}

impl BenchFullRegistry {
    pub const SPACE: usize = 8 + core::mem::size_of::<Self>();
}

pub fn bench_resolve_synthetic(_ctx: Context<BenchRegistry>, node_count: u16) -> Result<()> {
    require!(
        node_count > 0 && usize::from(node_count) <= BENCH_FULL_REGISTRY_MAX_NODES,
        MolphaError::InvalidNodeIndex
    );
    let mut ordered = Vec::with_capacity(usize::from(node_count));
    for _ in 0..node_count {
        ordered.push(([1u8; 32], [2u8; 32]));
    }
    require!(
        ordered.len() == usize::from(node_count),
        MolphaError::InvalidNodeIndex
    );
    Ok(())
}

/// Measures the exact production historical signer-account resolver.
pub fn bench_resolve_accounts(ctx: Context<BenchRegistry>, signers_bitmap: [u8; 32]) -> Result<()> {
    let registry = ctx.accounts.registry.load()?;
    let ordered = resolve_registry_signers(
        ctx.remaining_accounts,
        ctx.program_id,
        &registry,
        &signers_bitmap,
    )?;
    require!(
        ordered.len() == bitmap_load(&signers_bitmap).count_ones() as usize,
        MolphaError::InvalidNodeIndex
    );
    Ok(())
}

/// Isolates the cost of cloning and append/swap-removing an 8,208-byte snapshot.
pub fn bench_full_registry_transition(
    ctx: Context<BenchFullRegistryTransition>,
    old_version: u32,
    next_version: u32,
    mutation: u8,
    node: Pubkey,
    remove_index: u16,
) -> Result<()> {
    let old_registry = ctx.accounts.old_registry.load()?;
    require!(
        old_version == old_registry.version,
        MolphaError::InvalidRegistryVersion
    );
    require!(
        old_registry.version.checked_add(1) == Some(next_version),
        MolphaError::InvalidRegistryVersion
    );

    let mut new_registry = ctx.accounts.new_registry.load_init()?;
    new_registry.version = next_version;
    new_registry.node_count = old_registry.node_count;
    new_registry.redundancy_buffer = old_registry.redundancy_buffer;
    new_registry.bump = ctx.bumps.new_registry;
    new_registry.nodes.copy_from_slice(&old_registry.nodes);
    match mutation {
        0 => {
            require!(
                usize::from(new_registry.node_count) < BENCH_FULL_REGISTRY_MAX_NODES,
                MolphaError::NodeCapExceeded
            );
            let index = usize::from(new_registry.node_count);
            new_registry.nodes[index] = node.to_bytes();
            new_registry.node_count = new_registry
                .node_count
                .checked_add(1)
                .ok_or(error!(MolphaError::NodeCapExceeded))?;
        }
        1 => {
            let index = usize::from(remove_index);
            require!(
                index < usize::from(new_registry.node_count),
                MolphaError::InvalidNodeIndex
            );
            let tail = usize::from(new_registry.node_count - 1);
            new_registry.nodes[index] = new_registry.nodes[tail];
            new_registry.nodes[tail] = [0u8; 32];
            new_registry.node_count -= 1;
        }
        _ => return err!(MolphaError::InvalidNodeIndex),
    }
    Ok(())
}

/// Measures account-backed resolution against the benchmark snapshot layout.
pub fn bench_resolve_full_registry(
    ctx: Context<BenchResolveFullRegistry>,
    version: u32,
    signers_bitmap: [u8; 32],
) -> Result<()> {
    let registry = ctx.accounts.registry.load()?;
    require!(
        registry.version == version,
        MolphaError::InvalidRegistryVersion
    );
    let selected = bitmap_load(&signers_bitmap);
    require!(
        ctx.remaining_accounts.len() == selected.count_ones() as usize,
        MolphaError::MissingSignerAccount
    );

    let mut ordered = Vec::with_capacity(ctx.remaining_accounts.len());
    let mut cursor = 0usize;
    for_each_set_bit_u256(selected, |bit_pos| {
        let index = bit_pos as usize;
        require!(
            index < usize::from(registry.node_count),
            MolphaError::InvalidSignersBitmap
        );
        let account = ctx
            .remaining_accounts
            .get(cursor)
            .ok_or(error!(MolphaError::MissingSignerAccount))?;
        cursor += 1;
        require_keys_eq!(
            account.key(),
            Pubkey::new_from_array(registry.nodes[index]),
            MolphaError::MissingSignerAccount
        );
        require_keys_eq!(
            *account.owner,
            *ctx.program_id,
            MolphaError::MissingSignerAccount
        );
        let data = account.try_borrow_data()?;
        let node = Node::try_deserialize(&mut &data[..])
            .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
        ordered.push((node.secp256k1_pubkey_x, node.secp256k1_pubkey_y));
        Ok(())
    })?;
    require!(
        ordered.len() == ctx.remaining_accounts.len(),
        MolphaError::InvalidNodeIndex
    );
    Ok(())
}

#[derive(Accounts)]
pub struct BenchRegistry<'info> {
    pub registry: AccountLoader<'info, Registry>,
}

#[derive(Accounts)]
#[instruction(old_version: u32, next_version: u32)]
pub struct BenchFullRegistryTransition<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        seeds = [BENCH_FULL_REGISTRY_SEED_PREFIX, old_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub old_registry: AccountLoader<'info, BenchFullRegistry>,
    #[account(
        init,
        payer = payer,
        space = BenchFullRegistry::SPACE,
        seeds = [BENCH_FULL_REGISTRY_SEED_PREFIX, next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, BenchFullRegistry>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(version: u32)]
pub struct BenchResolveFullRegistry<'info> {
    #[account(
        seeds = [BENCH_FULL_REGISTRY_SEED_PREFIX, version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, BenchFullRegistry>,
}
