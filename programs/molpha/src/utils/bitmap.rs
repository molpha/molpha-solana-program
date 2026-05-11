use solana_keccak_hasher::hashv;

use crate::error::VerifyError;

/// Max derivation attempts (bounded compute); Solidity loop is unbounded.
const DERIVE_GROUP_BITMAP_MAX_ATTEMPTS: u64 = 65_536;

/// Solidity `uint256` layout for bitmaps: bit `pos` has weight `1 << pos`, `pos == 0` is the integer LSB (byte 31, bit 0).
#[inline]
pub fn evm_bitmap_bit_set(bitmap: &[u8; 32], pos: usize) -> bool {
    debug_assert!(pos < 256);
    let byte = 31 - (pos >> 3);
    let bit = pos & 7;
    (bitmap[byte] >> bit) & 1 == 1
}

#[inline]
pub fn evm_bitmap_set_bit(bitmap: &mut [u8; 32], pos: usize) {
    debug_assert!(pos < 256);
    let byte = 31 - (pos >> 3);
    let bit = pos & 7;
    bitmap[byte] |= 1 << bit;
}

pub fn bitmap_popcount_evm(bitmap: &[u8; 32]) -> u32 {
    bitmap.iter().map(|b| b.count_ones()).sum()
}

/// Bitmap **bit index** of the signer at 0-based rank `pos` when signers are ordered by ascending
/// bit index (same order as EVM `Validator.verify` combines pubkeys).
///
/// `pos` is the signer’s position in that list (`0` = first / lowest bit, `1` = second, …), not the
/// bit index itself. For the latter, use [`evm_bitmap_bit_set`].
///
/// Returns `None` if the bitmap has fewer than `pos + 1` signers.
pub fn get_index(bitmap: &[u8; 32], pos: usize) -> Option<usize> {
    let mut seen = 0usize;
    for bit_pos in 0..256 {
        if evm_bitmap_bit_set(bitmap, bit_pos) {
            if seen == pos {
                return Some(bit_pos);
            }
            seen += 1;
        }
    }
    None
}

fn u256_be_mod_bytes(hash: &[u8; 32], modulus: u32) -> u32 {
    if modulus == 0 {
        return 0;
    }
    let mut rem = 0u64;
    for byte in hash.iter() {
        rem = ((rem << 8) | (*byte as u64)) % (modulus as u64);
    }
    rem as u32
}

fn u32_as_u256_be(x: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[28..32].copy_from_slice(&x.to_be_bytes());
    out
}

fn u64_as_u256_be(x: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&x.to_be_bytes());
    out
}

/// Port of `NodeGroupBitmapLib.derive` from the EVM reference.
pub fn derive_group_bitmap(
    seed: &[u8; 32],
    round: u32,
    node_count: u32,
    group_size: u32,
) -> Result<[u8; 32], VerifyError> {
    if node_count == 0 {
        return Err(VerifyError::GroupBitmapDerivationFailed);
    }
    if group_size > node_count {
        return Err(VerifyError::GroupBitmapDerivationFailed);
    }
    if group_size == 0 {
        return Ok([0u8; 32]);
    }
    if group_size == node_count {
        if node_count == 256 {
            return Ok([0xFFu8; 32]);
        }
        let mut bm = [0u8; 32];
        for pos in 0..node_count as usize {
            evm_bitmap_set_bit(&mut bm, pos);
        }
        return Ok(bm);
    }

    let mut bitmap = [0u8; 32];
    let mut selected = 0u32;
    let mut attempt = 0u64;
    let round_word = u32_as_u256_be(round);

    while selected < group_size {
        if attempt >= DERIVE_GROUP_BITMAP_MAX_ATTEMPTS {
            return Err(VerifyError::GroupBitmapDerivationFailed);
        }

        let mut preimage = [0u8; 96];
        preimage[..32].copy_from_slice(seed);
        preimage[32..64].copy_from_slice(&round_word);
        preimage[64..96].copy_from_slice(&u64_as_u256_be(attempt));

        let h = hashv(&[&preimage]).to_bytes();
        let pos = u256_be_mod_bytes(&h, node_count) as usize;

        if !evm_bitmap_bit_set(&bitmap, pos) {
            evm_bitmap_set_bit(&mut bitmap, pos);
            selected += 1;
        }

        attempt += 1;
    }

    Ok(bitmap)
}

pub fn bitmap_is_subset(sub: &[u8; 32], sup: &[u8; 32]) -> bool {
    sub.iter().zip(sup.iter()).all(|(a, b)| *a & !*b == 0)
}

pub fn validate_bitmap_upper_bits_clear(
    bitmap: &[u8; 32],
    node_count: u32,
) -> Result<(), VerifyError> {
    if node_count > 256 {
        return Err(VerifyError::InvalidSignersBitmap);
    }
    for pos in (node_count as usize)..256 {
        if evm_bitmap_bit_set(bitmap, pos) {
            return Err(VerifyError::InvalidSignersBitmap);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_index_orders_signers_by_ascending_bit() {
        // EVM uint256 `7` => bits 0,1,2 set.
        let bm: [u8; 32] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0x07,
        ];
        assert_eq!(get_index(&bm, 0), Some(0));
        assert_eq!(get_index(&bm, 1), Some(1));
        assert_eq!(get_index(&bm, 2), Some(2));
        assert_eq!(get_index(&bm, 3), None);
    }

    #[test]
    fn get_index_skips_gaps() {
        let mut bm = [0u8; 32];
        evm_bitmap_set_bit(&mut bm, 5);
        evm_bitmap_set_bit(&mut bm, 10);
        assert_eq!(get_index(&bm, 0), Some(5));
        assert_eq!(get_index(&bm, 1), Some(10));
        assert_eq!(get_index(&bm, 2), None);
    }
}
