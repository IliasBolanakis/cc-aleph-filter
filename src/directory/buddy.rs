//! Buddy-block allocation, fingerprint bit-sacrifice, and migration routines.

use crate::block::layout::Block;
use crate::block::packing::{fingerprint_len, unpack_fingerprint, PackingError};

/// Splits a saturated block into two buddy blocks at `local_depth + 1`.
///
/// Iterates across all resident quotient runs in `original`:
/// 1. Extracts each resident fingerprint.
/// 2. Reads $b_{\text{MSB}} = \lfloor f / 2^{F_d - 1} \rfloor$.
/// 3. Computes the truncated fingerprint $f' = f \pmod{2^{F_d - 1}}$[cite: 1].
/// 4. Routes items with $b_{\text{MSB}} = 0$ to `block_0` and $b_{\text{MSB}} = 1$ to `block_1`[cite: 1].
/// 5. Advances the generation count on both daughter blocks.
///
/// # Arguments
/// * `original` - Saturated source block.
/// * `local_depth` - Current local depth $d$ of the block before splitting.
/// * `base_fp_bits` - Initial fingerprint bit length at depth 0.
///
/// # Errors
/// Returns [`PackingError`] if packing limits are exceeded.
pub fn split_block(
    original: &Block,
    local_depth: usize,
    base_fp_bits: usize,
) -> Result<(Block, Block), PackingError> {
    let current_fp_len = fingerprint_len(base_fp_bits, local_depth);
    if current_fp_len <= 1 {
        return Err(PackingError::InvalidBitLength {
            bit_len: current_fp_len,
            max_allowed: 64,
        });
    }

    let next_fp_len = current_fp_len - 1;
    let next_generation = original.generation() + 1;

    let mut block_0 = Block::new();
    let mut block_1 = Block::new();

    block_0.set_generation(next_generation);
    block_1.set_generation(next_generation);

    for q in 0..Block::SLOTS_PER_BLOCK {
        if let Some((item_offset, run_len)) = original.run_bounds(q) {
            for i in 0..run_len {
                let bit_offset = (item_offset + i) * current_fp_len;
                let fp = unpack_fingerprint(original.payload(), bit_offset, current_fp_len)?;

                // Extract MSB to route between primary and buddy blocks
                let b_msb = (fp >> (current_fp_len - 1)) & 1;
                // Shed the MSB for the new generation
                let shortened_fp = fp & ((1u64 << next_fp_len) - 1);

                if b_msb == 0 {
                    block_0.insert(q, shortened_fp, next_fp_len)?;
                } else {
                    block_1.insert(q, shortened_fp, next_fp_len)?;
                }
            }
        }
    }

    debug_assert_eq!(
        block_0.count() + block_1.count(),
        original.count(),
        "Item count invariant violated during buddy split"
    );

    Ok((block_0, block_1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_block_item_distribution() {
        let mut original = Block::new();
        let base_fp = 8;
        let depth = 0;
        let fp_len = fingerprint_len(base_fp, depth); // 8 bits

        // Insert elements with MSB = 0 into slot 3
        original.insert(3, 0b0010_1010, fp_len).unwrap(); // Shortens to 0b010_1010
        original.insert(3, 0b0100_0001, fp_len).unwrap(); // Shortens to 0b100_0001

        // Insert element with MSB = 1 into slot 3
        original.insert(3, 0b1011_0011, fp_len).unwrap(); // Shortens to 0b011_0011

        // Insert element with MSB = 1 into slot 15
        original.insert(15, 0b1110_0000, fp_len).unwrap(); // Shortens to 0b110_0000

        assert_eq!(original.count(), 4);

        let (b0, b1) = split_block(&original, depth, base_fp).unwrap();
        let next_fp = fp_len - 1; // 7 bits

        // Verify counts
        assert_eq!(b0.count(), 2);
        assert_eq!(b1.count(), 2);

        // Verify b0 items (MSB == 0)
        assert!(b0.query(3, 0b010_1010, next_fp));
        assert!(b0.query(3, 0b100_0001, next_fp));
        assert!(!b0.query(3, 0b011_0011, next_fp));

        // Verify b1 items (MSB == 1)
        assert!(b1.query(3, 0b011_0011, next_fp));
        assert!(b1.query(15, 0b110_0000, next_fp));
        assert!(!b1.query(3, 0b010_1010, next_fp));

        // Verify generation increment
        assert_eq!(b0.generation(), 1);
        assert_eq!(b1.generation(), 1);
    }
}
