//! 128-bit xxHash3 decomposition, bit extraction, and directory routing routines.

use xxhash_rust::xxh3::xxh3_128;

use crate::block::layout::Block;
use crate::block::packing::fingerprint_len;

/// Default initial fingerprint bit-width before table doubling steps.
pub const DEFAULT_BASE_FP_BITS: usize = 16;

/// Number of bit positions allocated to intra-block quotient addressing ($2^5 = 32$).
pub const INTRA_BLOCK_QUOTIENT_BITS: usize = 5;

// Compile-time verification of intra-block slot alignment.
const _: () = {
    assert!(1 << INTRA_BLOCK_QUOTIENT_BITS == Block::SLOTS_PER_BLOCK);
};

/// Decomposition of a 128-bit hash into directory routing indices and fingerprint bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyDecomposition {
    /// Slot index inside the target 64-byte block ($0 \le q < 32$).
    pub intra_quotient: usize,
    /// Physical directory bucket index at the given local depth.
    pub block_index: usize,
    /// Fingerprint scalar of bit-width `fp_bit_len`.
    pub fingerprint: u64,
    /// Active bit-width of the fingerprint.
    pub fp_bit_len: usize,
}

/// Hashes a byte key using xxHash3-128 and extracts the quotient and fingerprint parameters.
///
/// # Arguments
/// * `key` - Arbitrary byte slice to hash.
/// * `local_depth` - Local depth $d$ of the target block.
/// * `base_fp_bits` - Initial fingerprint bit width at depth $d = 0$.
///
/// # Returns
/// A [`KeyDecomposition`] struct containing the intra-block slot, block index, and fingerprint.
#[inline]
#[must_use]
pub fn decompose_key(key: &[u8], local_depth: usize, base_fp_bits: usize) -> KeyDecomposition {
    let hash = xxh3_128(key);
    decompose_hash(hash, local_depth, base_fp_bits)
}

/// Decomposes an existing 128-bit hash integer into block routing and fingerprint fields.
#[inline]
#[must_use]
pub fn decompose_hash(hash: u128, local_depth: usize, base_fp_bits: usize) -> KeyDecomposition {
    // 1. Lowest 5 bits: intra-block quotient slot [0..32)
    let intra_quotient = (hash as usize) & (Block::SLOTS_PER_BLOCK - 1);

    // 2. Shift past intra-block bits to enter directory routing & fingerprint space
    let rest = hash >> INTRA_BLOCK_QUOTIENT_BITS;

    // 3. Next `local_depth` bits: block index
    let block_index = if local_depth == 0 {
        0
    } else {
        (rest as usize) & ((1usize << local_depth) - 1)
    };

    // 4. Calculate fingerprint length at this local depth: F_d = base - d
    let fp_bit_len = fingerprint_len(base_fp_bits, local_depth);

    // 5. Extract fingerprint starting at bit `local_depth`.
    // The bits are reversed so that the bit at offset `local_depth` becomes the MSB of the fingerprint,
    // ensuring alignment with the buddy-split bit-sacrifice operation.
    let fingerprint = if fp_bit_len == 0 {
        0
    } else {
        let raw_chunk = ((rest >> local_depth) as u64) & ((1u64 << fp_bit_len) - 1);
        raw_chunk.reverse_bits() >> (64 - fp_bit_len)
    };

    KeyDecomposition {
        intra_quotient,
        block_index,
        fingerprint,
        fp_bit_len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intra_block_quotient_bounds() {
        for i in 0..1000 {
            let key = format!("benchmark_key_{i}");
            let decomp = decompose_key(key.as_bytes(), 0, 16);
            assert!(
                decomp.intra_quotient < Block::SLOTS_PER_BLOCK,
                "Intra-block quotient must be < 32"
            );
        }
    }

    #[test]
    fn test_bit_sacrifice_mathematical_continuity() {
        // Verify that the MSB of fingerprint at depth d becomes the routing bit at depth d + 1,
        // and the remaining bits match the fingerprint at depth d + 1.
        let key = b"verification_key_for_bit_sacrifice";
        let base_fp = 16;

        for depth in 0..10 {
            let decomp_d = decompose_key(key, depth, base_fp);
            let decomp_next = decompose_key(key, depth + 1, base_fp);

            // Intra-block quotient must never change across expansions
            assert_eq!(decomp_d.intra_quotient, decomp_next.intra_quotient);

            // Fingerprint length must decrease strictly by 1
            assert_eq!(decomp_next.fp_bit_len, decomp_d.fp_bit_len - 1);

            // Extract MSB of fingerprint at depth d
            let msb = (decomp_d.fingerprint >> (decomp_d.fp_bit_len - 1)) & 1;

            // Shortened fingerprint at depth d
            let shortened_fp = decomp_d.fingerprint & ((1u64 << (decomp_d.fp_bit_len - 1)) - 1);

            // The shortened fingerprint must match the fingerprint directly generated at depth d + 1
            assert_eq!(
                shortened_fp, decomp_next.fingerprint,
                "Shortened fingerprint mismatch at depth {depth}"
            );

            // The routing bit at depth d + 1 must match the extracted MSB
            let routing_bit = ((decomp_next.block_index >> depth) & 1) as u64;
            assert_eq!(
                msb, routing_bit,
                "Routing bit mismatch at depth {depth}: msb={msb}, routing_bit={routing_bit}"
            );
        }
    }
}