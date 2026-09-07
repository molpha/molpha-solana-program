//! Bump allocator with a 256 KiB heap — enough for large-coalition punishment filings.
//!
//! The default 32 KiB heap is exhausted when many `Node` accounts are deserialized in one
//! instruction; production callers also raise the frame via `RequestHeapFrame`.

#[cfg(target_os = "solana")]
use anchor_lang::solana_program::entrypoint::HEAP_START_ADDRESS;
#[cfg(target_os = "solana")]
use std::{alloc::Layout, mem::size_of, ptr::null_mut};

#[cfg(target_os = "solana")]
pub const HEAP_LENGTH: usize = 256 * 1024;

#[cfg(target_os = "solana")]
pub struct BumpAllocator;

#[cfg(target_os = "solana")]
unsafe impl std::alloc::GlobalAlloc for BumpAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        const POS_PTR: *mut usize = HEAP_START_ADDRESS as *mut usize;
        const TOP_ADDRESS: usize = HEAP_START_ADDRESS as usize + HEAP_LENGTH;
        const BOTTOM_ADDRESS: usize = HEAP_START_ADDRESS as usize + size_of::<usize>();

        let mut pos = *POS_PTR;
        if pos == 0 {
            pos = BOTTOM_ADDRESS;
        }

        pos = (pos + layout.align() - 1) & !(layout.align() - 1);
        let next_pos = pos.saturating_add(layout.size());
        if next_pos > TOP_ADDRESS {
            return null_mut();
        }

        *POS_PTR = next_pos;
        pos as *mut u8
    }

    #[inline]
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[cfg(all(
    target_os = "solana",
    feature = "custom-heap",
    not(feature = "no-entrypoint")
))]
#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator;
