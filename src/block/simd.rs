//! SIMD-accelerated intra-block search and bit-manipulation primitives.
//!
//! Provides hardware-accelerated scanning over 64-byte cacheline blocks using
//! AVX2 vector instructions and BMI2 bit-manipulation intrinsics (_pext_u64, _pdep_u64).

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{
    __m128i, __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
    _mm256_set1_epi8, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    _pdep_u64,
};

use crate::block::layout::PAYLOAD_SIZE_BYTES;
use crate::block::packing::read_bits;

/// Computes the zero-indexed position of the `k`-th set bit (1-bit) in `mask`.
///
/// If fewer than `k + 1` bits are set, returns `None`.
/// Uses BMI2 `_pdep_u64` on supported x86_64 targets for $O(1)$ cycle evaluation.
#[inline]
#[must_use]
pub fn select64(mask: u64, k: u32) -> Option<u32> {
    if k >= 64 || mask.count_ones() <= k {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("bmi2") {
            // Safety: Target feature verified at runtime; mask and bit shift are valid.
            unsafe {
                let dep = _pdep_u64(1u64 << k, mask);
                return Some(dep.trailing_zeros());
            }
        }
    }

    select64_scalar(mask, k)
}

/// Scalar fallback for `select64` using iterative broadword bit-clearing.
#[inline]
#[must_use]
pub fn select64_scalar(mut mask: u64, mut k: u32) -> Option<u32> {
    while mask != 0 {
        let tz = mask.trailing_zeros();
        if k == 0 {
            return Some(tz);
        }
        k -= 1;
        mask &= mask - 1; // Clear least-significant set bit
    }
    None
}

/// Returns the number of set bits (1-bits) strictly before `bit_idx` in `mask`.
#[inline]
#[must_use]
pub const fn rank64(mask: u64, bit_idx: u32) -> u32 {
    if bit_idx == 0 {
        0
    } else if bit_idx >= 64 {
        mask.count_ones()
    } else {
        let bitmask = (1u64 << bit_idx) - 1;
        (mask & bitmask).count_ones()
    }
}

/// Scans the entire 48-byte block payload for occurrences of `target_byte` using AVX2 and SSE.
///
/// Returns a 64-bit integer where bit `i` is set if and only if `payload[i] == target_byte`.
/// Bits in positions `[48..64]` are guaranteed to be zero.
#[must_use]
pub fn scan_payload_bytes(payload: &[u8; PAYLOAD_SIZE_BYTES], target_byte: u8) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // Safety: payload is a valid 48-byte aligned array; AVX2 is available.
            return unsafe { scan_payload_avx2(payload, target_byte) };
        }
    }

    scan_payload_scalar(payload, target_byte)
}

/// AVX2 + SSE vectorized implementation of 48-byte payload scan.
///
/// # Safety
/// The caller must ensure that the CPU supports AVX2 intrinsics.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn scan_payload_avx2(payload: &[u8; PAYLOAD_SIZE_BYTES], target_byte: u8) -> u64 {
    let ptr = payload.as_ptr();

    // 1. Process bytes 0..32 using AVX2 (256-bit register)
    let chunk256 = unsafe { _mm256_loadu_si256(ptr as *const __m256i) };
    let target256 = _mm256_set1_epi8(target_byte as i8);
    let cmp256 = _mm256_cmpeq_epi8(chunk256, target256);
    let mask_low32 = _mm256_movemask_epi8(cmp256) as u32 as u64;

    // 2. Process bytes 32..48 using SSE (128-bit register)
    let chunk128 = unsafe { _mm_loadu_si128(ptr.add(32) as *const __m128i) };
    let target128 = _mm_set1_epi8(target_byte as i8);
    let cmp128 = _mm_cmpeq_epi8(chunk128, target128);
    let mask_high16 = _mm_movemask_epi8(cmp128) as u16 as u64;

    mask_low32 | (mask_high16 << 32)
}

/// Scalar fallback for 48-byte payload byte scan.
#[inline]
#[must_use]
pub fn scan_payload_scalar(payload: &[u8; PAYLOAD_SIZE_BYTES], target_byte: u8) -> u64 {
    let mut mask = 0u64;
    for (i, &byte) in payload.iter().enumerate() {
        if byte == target_byte {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// Searches for `target_fp` within a contiguous run of fingerprints in the payload.
///
/// # Arguments
/// * `payload` - 48-byte packed block payload.
/// * `start_bit_offset` - Bit offset of the first fingerprint in the run.
/// * `num_fingerprints` - Total number of fingerprints resident in this run.
/// * `bit_len` - Bit width of each fingerprint.
/// * `target_fp` - The target fingerprint scalar to match.
///
/// # Returns
/// Returns `Some(index_within_run)` if found, or `None` if absent.
pub fn find_fingerprint_in_run(
    payload: &[u8; PAYLOAD_SIZE_BYTES],
    start_bit_offset: usize,
    num_fingerprints: usize,
    bit_len: usize,
    target_fp: u64,
) -> Option<usize> {
    if num_fingerprints == 0 || bit_len == 0 {
        return None;
    }

    // Fast path: byte-aligned 8-bit fingerprints using SIMD comparison
    if bit_len == 8 && (start_bit_offset % 8 == 0) && target_fp <= 0xFF {
        let start_byte = start_bit_offset / 8;
        let end_byte = (start_byte + num_fingerprints).min(PAYLOAD_SIZE_BYTES);

        let match_mask = scan_payload_bytes(payload, target_fp as u8);

        // Filter the match mask to the target range [start_byte..end_byte]
        let range_len = end_byte - start_byte;
        let range_mask = if range_len == 64 {
            u64::MAX
        } else {
            (1u64 << range_len) - 1
        };

        let active_matches = (match_mask >> start_byte) & range_mask;
        if active_matches != 0 {
            return Some(active_matches.trailing_zeros() as usize);
        }
        return None;
    }

    // Unaligned / variable-length bit scan
    for i in 0..num_fingerprints {
        let offset = start_bit_offset + (i * bit_len);
        if let Ok(fp) = read_bits(payload, offset, bit_len) {
            if fp == target_fp {
                return Some(i);
            }
        } else {
            break;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rank64_bounds() {
        let mask = 0b1011_0001u64;
        assert_eq!(rank64(mask, 0), 0);
        assert_eq!(rank64(mask, 1), 1);
        assert_eq!(rank64(mask, 2), 1);
        assert_eq!(rank64(mask, 4), 1);
        assert_eq!(rank64(mask, 5), 2);
        assert_eq!(rank64(mask, 64), 4);
    }

    #[test]
    fn test_select64_matches_scalar() {
        let test_masks = [
            0u64,
            1u64,
            0x8000_0000_0000_0000u64,
            0xAAAA_AAAA_AAAA_AAAAu64,
            0xFFFF_FFFF_FFFF_FFFFu64,
            0x1020_4080_0402_0100u64,
        ];

        for &mask in &test_masks {
            let count = mask.count_ones();
            for k in 0..count {
                let scalar = select64_scalar(mask, k);
                let accelerated = select64(mask, k);
                assert_eq!(
                    scalar, accelerated,
                    "Mismatch for mask {mask:#018x} at rank {k}"
                );
            }
            assert_eq!(select64_scalar(mask, count), None);
            assert_eq!(select64(mask, count), None);
        }
    }

    #[test]
    fn test_simd_vs_scalar_payload_scan() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        payload[0] = 0x42;
        payload[15] = 0x42;
        payload[31] = 0x42;
        payload[32] = 0x42;
        payload[47] = 0x42;

        let scalar_result = scan_payload_scalar(&payload, 0x42);
        let simd_result = scan_payload_bytes(&payload, 0x42);

        assert_eq!(scalar_result, simd_result);
        assert_eq!(simd_result & (1u64 << 0), 1u64 << 0);
        assert_eq!(simd_result & (1u64 << 15), 1u64 << 15);
        assert_eq!(simd_result & (1u64 << 31), 1u64 << 31);
        assert_eq!(simd_result & (1u64 << 32), 1u64 << 32);
        assert_eq!(simd_result & (1u64 << 47), 1u64 << 47);
        assert_eq!(simd_result & !((1u64 << 48) - 1), 0);
    }

    #[test]
    fn test_find_fingerprint_in_run_8bit() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        payload[2] = 0xAA;
        payload[3] = 0xBB;
        payload[4] = 0xCC;

        // Run starting at byte offset 2 (bit 16), containing 3 elements
        assert_eq!(find_fingerprint_in_run(&payload, 16, 3, 8, 0xAA), Some(0));
        assert_eq!(find_fingerprint_in_run(&payload, 16, 3, 8, 0xBB), Some(1));
        assert_eq!(find_fingerprint_in_run(&payload, 16, 3, 8, 0xCC), Some(2));
        assert_eq!(find_fingerprint_in_run(&payload, 16, 3, 8, 0xDD), None);
    }

    #[test]
    fn test_find_fingerprint_in_run_variable_bits() {
        use crate::block::packing::pack_fingerprint;

        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        let bit_len = 5;
        let fps = [0b10101u64, 0b01010u64, 0b11100u64];

        let mut offset = 13; // Unaligned start
        for &fp in &fps {
            pack_fingerprint(&mut payload, offset, offset, bit_len, fp).unwrap();
            offset += bit_len;
        }

        assert_eq!(find_fingerprint_in_run(&payload, 13, 3, bit_len, 0b10101), Some(0));
        assert_eq!(find_fingerprint_in_run(&payload, 13, 3, bit_len, 0b01010), Some(1));
        assert_eq!(find_fingerprint_in_run(&payload, 13, 3, bit_len, 0b11100), Some(2));
        assert_eq!(find_fingerprint_in_run(&payload, 13, 3, bit_len, 0b00000), None);
    }
}