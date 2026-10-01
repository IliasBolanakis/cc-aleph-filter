//! Cacheline-aligned memory layout for isolated 64-byte blocks.

use core::mem::{align_of, size_of};

use crate::block::packing::{
    pack_fingerprint, remove_fingerprint, PackingError, PAYLOAD_CAPACITY_BITS,
};
use crate::block::simd::{find_fingerprint_in_run, rank64, select64};

/// Total size of an individual block in bytes, matching an x86 L1-D cacheline[cite: 1].
pub const BLOCK_SIZE_BYTES: usize = 64;

/// Capacity of the packed fingerprint region in bytes[cite: 1].
pub const PAYLOAD_SIZE_BYTES: usize = 48;

/// Hardware-aligned 64-byte block storing quotient runs, bitmasks, and fingerprints[cite: 1, 4].
///
/// Layout:
/// - `header`: 8 bytes (run tracking, element count, and local generation)[cite: 1, 4].
/// - `unary_bitmask`: 8 bytes (unary run boundary encoding)[cite: 1, 4].
/// - `payload`: 48 bytes (tightly packed variable-length fingerprint bits)[cite: 1, 4].
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// 64-bit control field encoding run markers and slot occupancy[cite: 4].
    pub header: u64,
    /// 64-bit unary bitmask delineating boundaries of quotient groups[cite: 4].
    pub unary_bitmask: u64,
    /// 48-byte packed storage for variable-length fingerprints[cite: 4].
    pub payload: [u8; PAYLOAD_SIZE_BYTES],
}

// Compile-time verification of layout invariants.
const _: () = {
    assert!(size_of::<Block>() == BLOCK_SIZE_BYTES);
    assert!(align_of::<Block>() == BLOCK_SIZE_BYTES);
};

impl Block {
    /// Number of quotient slots represented in the unary bitmask.
    ///
    /// With 32 slots (represented by 32 zero-delimiter bits) and up to 32 resident items
    /// (one-bits), the complete unary representation fits into `unary_bitmask: u64` (64 bits).
    pub const SLOTS_PER_BLOCK: usize = 32;

    /// Creates an empty, zero-initialized block[cite: 4].
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            header: 0,
            unary_bitmask: 0,
            payload: [0u8; PAYLOAD_SIZE_BYTES],
        }
    }

    /// Resets all header bits, unary bitmasks, and payload bytes to zero[cite: 4].
    #[inline]
    pub fn clear(&mut self) {
        self.header = 0;
        self.unary_bitmask = 0;
        self.payload.fill(0);
    }

    /// Returns a direct immutable slice view of the packed fingerprint payload[cite: 4].
    #[inline]
    #[must_use]
    pub fn payload(&self) -> &[u8; PAYLOAD_SIZE_BYTES] {
        &self.payload
    }

    /// Returns a direct mutable slice view of the packed fingerprint payload[cite: 4].
    #[inline]
    pub fn payload_mut(&mut self) -> &mut [u8; PAYLOAD_SIZE_BYTES] {
        &mut self.payload
    }

    /// Returns whether the block contains no elements[cite: 4].
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.header == 0 && self.unary_bitmask == 0
    }

    /// Returns the total number of items currently stored in this block.
    #[inline]
    #[must_use]
    pub const fn count(&self) -> usize {
        (self.header & 0xFFFF) as usize
    }

    /// Sets the item count in the header.
    #[inline]
    pub fn set_count(&mut self, count: usize) {
        self.header = (self.header & !0xFFFF) | ((count as u64) & 0xFFFF);
    }

    /// Returns the current bit-length generation of fingerprints in this block.
    #[inline]
    #[must_use]
    pub const fn generation(&self) -> usize {
        ((self.header >> 16) & 0xFF) as usize
    }

    /// Sets the bit-length generation in the header.
    #[inline]
    pub fn set_generation(&mut self, gen: usize) {
        self.header = (self.header & !(0xFF << 16)) | (((gen as u64) & 0xFF) << 16);
    }

    /// Locates the start index and length of a quotient run in $O(1)$ operations[cite: 1].
    ///
    /// In the unary bitmask, quotient slot delimiters are encoded as 0-bits and resident items
    /// are encoded as 1-bits. The $q$-th delimiter position corresponds to the $q$-th set bit
    /// in `!unary_bitmask`.
    #[must_use]
    pub fn run_bounds(&self, quotient: usize) -> Option<(usize, usize)> {
        if quotient >= Self::SLOTS_PER_BLOCK {
            return None;
        }

        let inv_mask = !self.unary_bitmask;
        let delim_pos = select64(inv_mask, quotient as u32)?;

        let start_pos = if quotient == 0 {
            0
        } else {
            select64(inv_mask, (quotient - 1) as u32)? + 1
        };

        let run_len = delim_pos.saturating_sub(start_pos) as usize;
        let item_offset = rank64(self.unary_bitmask, start_pos) as usize;

        Some((item_offset, run_len))
    }

    /// Queries whether `fingerprint` exists for `quotient` within this block[cite: 1].
    #[must_use]
    pub fn query(&self, quotient: usize, fingerprint: u64, fp_bit_len: usize) -> bool {
        let Some((item_offset, run_len)) = self.run_bounds(quotient) else {
            return false;
        };

        if run_len == 0 {
            return false;
        }

        let start_bit = item_offset * fp_bit_len;
        find_fingerprint_in_run(&self.payload, start_bit, run_len, fp_bit_len, fingerprint)
            .is_some()
    }

    /// Inserts `fingerprint` into `quotient`'s run inside this block[cite: 1].
    ///
    /// # Errors
    /// Returns [`PackingError::CapacityOverflow`] if the payload or unary mask capacity is exceeded.
    pub fn insert(
        &mut self,
        quotient: usize,
        fingerprint: u64,
        fp_bit_len: usize,
    ) -> Result<(), PackingError> {
        if quotient >= Self::SLOTS_PER_BLOCK {
            return Err(PackingError::OutOfBounds {
                bit_offset: quotient,
                bit_len: 1,
                capacity_bits: Self::SLOTS_PER_BLOCK,
            });
        }

        let count = self.count();
        let total_bits = count * fp_bit_len;

        if total_bits + fp_bit_len > PAYLOAD_CAPACITY_BITS {
            return Err(PackingError::CapacityOverflow {
                required_bits: total_bits + fp_bit_len,
                max_bits: PAYLOAD_CAPACITY_BITS,
            });
        }

        if Self::SLOTS_PER_BLOCK + count >= 64 {
            return Err(PackingError::CapacityOverflow {
                required_bits: Self::SLOTS_PER_BLOCK + count + 1,
                max_bits: 64,
            });
        }

        let (item_offset, run_len) = self.run_bounds(quotient).unwrap_or((count, 0));
        let insert_idx = item_offset + run_len;
        let bit_offset = insert_idx * fp_bit_len;

        // Pack the fingerprint into the 48-byte payload region
        pack_fingerprint(
            &mut self.payload,
            bit_offset,
            total_bits,
            fp_bit_len,
            fingerprint,
        )?;

        // Update unary bitmask: insert a 1-bit immediately before delimiter `quotient`
        let inv_mask = !self.unary_bitmask;
        let delim_pos =
            select64(inv_mask, quotient as u32).ok_or(PackingError::CapacityOverflow {
                required_bits: 64,
                max_bits: 64,
            })?;

        let low_mask = if delim_pos == 0 {
            0
        } else {
            (1u64 << delim_pos) - 1
        };
        let low_bits = self.unary_bitmask & low_mask;
        let high_bits = (self.unary_bitmask & !low_mask) << 1;
        self.unary_bitmask = low_bits | (1u64 << delim_pos) | high_bits;

        self.set_count(count + 1);
        Ok(())
    }

    /// Removes `fingerprint` from `quotient`'s run inside this block.
    ///
    /// # Errors
    /// Returns [`PackingError`] if payload shifts fail.
    pub fn remove(
        &mut self,
        quotient: usize,
        fingerprint: u64,
        fp_bit_len: usize,
    ) -> Result<bool, PackingError> {
        let Some((item_offset, run_len)) = self.run_bounds(quotient) else {
            return Ok(false);
        };

        if run_len == 0 {
            return Ok(false);
        }

        let start_bit = item_offset * fp_bit_len;
        let match_idx =
            find_fingerprint_in_run(&self.payload, start_bit, run_len, fp_bit_len, fingerprint);

        let Some(relative_idx) = match_idx else {
            return Ok(false);
        };

        let target_item_idx = item_offset + relative_idx;
        let count = self.count();
        let total_bits = count * fp_bit_len;

        // Shift remaining payload bits left
        remove_fingerprint(
            &mut self.payload,
            target_item_idx * fp_bit_len,
            total_bits,
            fp_bit_len,
        )?;

        // Remove the corresponding 1-bit from the unary bitmask
        let inv_mask = !self.unary_bitmask;
        let start_pos = if quotient == 0 {
            0
        } else {
            select64(inv_mask, (quotient - 1) as u32).unwrap() + 1
        };
        let bit_to_remove = start_pos + relative_idx as u32;

        let low_mask = if bit_to_remove == 0 {
            0
        } else {
            (1u64 << bit_to_remove) - 1
        };
        let low_bits = self.unary_bitmask & low_mask;
        let high_bits = (self.unary_bitmask >> (bit_to_remove + 1)) << bit_to_remove;
        self.unary_bitmask = low_bits | high_bits;

        self.set_count(count - 1);
        Ok(true)
    }
}

impl Default for Block {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_alignment_and_size() {
        assert_eq!(size_of::<Block>(), 64);
        assert_eq!(align_of::<Block>(), 64);

        let block = Block::new();
        let ptr = &block as *const Block as usize;
        assert_eq!(ptr % 64, 0, "Block must be 64-byte aligned on the stack");
    }

    #[test]
    fn test_block_initialization_and_clear() {
        let mut block = Block::new();
        assert!(block.is_empty());

        block.header = 0xDEAD_BEEF_CAFE_BABE;
        block.unary_bitmask = 0x0123_4567_89AB_CDEF;
        block.payload[0] = 0xFF;
        block.payload[PAYLOAD_SIZE_BYTES - 1] = 0xAA;

        assert!(!block.is_empty());

        block.clear();
        assert!(block.is_empty());
        assert_eq!(block.header, 0);
        assert_eq!(block.unary_bitmask, 0);
        assert_eq!(block.payload, [0u8; PAYLOAD_SIZE_BYTES]);
    }

    #[test]
    fn test_block_insert_and_query_integration() {
        let mut block = Block::new();
        let fp_len = 8;

        assert_eq!(block.count(), 0);
        assert!(!block.query(5, 0xAB, fp_len));

        // Insert into quotient slot 5
        block.insert(5, 0xAB, fp_len).unwrap();
        assert_eq!(block.count(), 1);
        assert!(block.query(5, 0xAB, fp_len));
        assert!(!block.query(5, 0xAC, fp_len));
        assert!(!block.query(6, 0xAB, fp_len));

        // Insert a second element into quotient slot 5
        block.insert(5, 0xCD, fp_len).unwrap();
        assert_eq!(block.count(), 2);
        assert!(block.query(5, 0xAB, fp_len));
        assert!(block.query(5, 0xCD, fp_len));

        // Insert an element into quotient slot 12
        block.insert(12, 0xEF, fp_len).unwrap();
        assert_eq!(block.count(), 3);
        assert!(block.query(12, 0xEF, fp_len));
        assert!(block.query(5, 0xAB, fp_len));
    }

    #[test]
    fn test_block_remove_integration() {
        let mut block = Block::new();
        let fp_len = 8;

        block.insert(5, 0x11, fp_len).unwrap();
        block.insert(5, 0x22, fp_len).unwrap();
        block.insert(8, 0x33, fp_len).unwrap();
        assert_eq!(block.count(), 3);

        // Remove non-existent key
        assert!(!block.remove(5, 0x99, fp_len).unwrap());
        assert_eq!(block.count(), 3);

        // Remove 0x11 from quotient 5
        assert!(block.remove(5, 0x11, fp_len).unwrap());
        assert_eq!(block.count(), 2);
        assert!(!block.query(5, 0x11, fp_len));
        assert!(block.query(5, 0x22, fp_len));
        assert!(block.query(8, 0x33, fp_len));

        // Remove 0x22 from quotient 5
        assert!(block.remove(5, 0x22, fp_len).unwrap());
        assert_eq!(block.count(), 1);
        assert!(!block.query(5, 0x22, fp_len));
        assert!(block.query(8, 0x33, fp_len));

        // Remove final item
        assert!(block.remove(8, 0x33, fp_len).unwrap());
        assert_eq!(block.count(), 0);
        assert!(!block.query(8, 0x33, fp_len));
    }
}
