//! Dev-only CU probe for `utils::bitmap::derive_group_bitmap` (build with `--features cu-bench`).

use anchor_lang::prelude::*;

use crate::utils::bitmap::derive_group_bitmap;

#[derive(Accounts)]
pub struct BenchDeriveGroupBitmap {}

pub fn bench_noop(_ctx: Context<BenchDeriveGroupBitmap>) -> Result<()> {
    Ok(())
}

pub fn bench_derive_group_bitmap(
    _ctx: Context<BenchDeriveGroupBitmap>,
    seed: [u8; 32],
    node_count: u32,
    group_size: u32,
) -> Result<()> {
    let _bitmap = derive_group_bitmap(&seed, node_count, group_size)?;
    Ok(())
}
