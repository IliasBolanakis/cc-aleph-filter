//! Bit-level packing and extraction routines for variable-length fingerprints.
//!
//! Aleph filters maintain constant space during directory doubling by shaving
//! one bit of precision from resident fingerprints ($F_i = F - i$). Because
//! fingerprints do not align to byte boundaries, this module implements unaligned
//! bitstream manipulation over the 48-byte block payload region.

use core::fmt;

use crate::block::layout::PAYLOAD_SIZE_BYTES;

/// Total capacity of the packed fingerprint region in bits (48 bytes * 8 bits = 384 bits).
pub const PAYLOAD_CAPACITY_BITS: usize = PAYLOAD_SIZE_BYTES * 8;

/// Errors that can occur during bitstream packing and extraction operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackingError {
    /// Attempted to write beyond the 384-bit block capacity.
    CapacityOverflow {
        /// Number of bits required for the operation.
        required_bits: usize,
        /// Maximum bits available in the payload.
        max_bits: usize,
    },
    /// Fingerprint bit-length exceeds the supported scalar width (64 bits).
    InvalidBitLength {
        /// Requested bit length.
        bit_len: usize,
        /// Maximum permissible length.
        max_allowed: usize,
    },
    /// Bit range falls outside the specified slice bounds.
    OutOfBounds {
        /// Target start bit offset.
        bit_offset: usize,
        /// Length of the target slice in bits.
        bit_len: usize,
        /// Total capacity of the buffer in bits.
        capacity_bits: usize,
    },
}

impl fmt::Display for PackingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CapacityOverflow { required_bits, max_bits } => {
                write!(
                    f,
                    "Payload capacity overflow: required {required_bits} bits, maximum capacity is {max_bits} bits"
                )
            }
            Self::InvalidBitLength { bit_len, max_allowed } => {
                write!(
                    f,
                    "Invalid bit length {bit_len}: maximum supported scalar length is {max_allowed} bits"
                )
            }
            Self::OutOfBounds { bit_offset, bit_len, capacity_bits } => {
                write!(
                    f,
                    "Bit access out of bounds: range [{bit_offset}..{}) exceeds capacity {capacity_bits} bits",
                    bit_offset + bit_len
                )
            }
        }
    }
}

impl std::error::Error for PackingError {}

/// Calculates the target fingerprint bit length for a given filter doubling generation.
///
/// Under the Aleph shrinking rule, each doubling step $i$ reduces the fingerprint
/// width by one bit: $F_i = F_{\text{base}} - i$.
#[inline]
#[must_use]
pub const fn fingerprint_len(base_bits: usize, generation: usize) -> usize {
    base_bits.saturating_sub(generation)
}

/// Reads an unaligned bit sequence up to 64 bits from an arbitrary bit offset.
///
/// # Arguments
/// * `buf` - Byte slice containing the bitstream.
/// * `bit_offset` - Zero-indexed bit position to begin reading.
/// * `bit_len` - Number of bits to read (must be in range `[0, 64]`).
///
/// # Errors
/// Returns [`PackingError::InvalidBitLength`] if `bit_len > 64`, or
/// [`PackingError::OutOfBounds`] if the target range exceeds `buf.len() * 8`.
#[inline]
pub fn read_bits(buf: &[u8], bit_offset: usize, bit_len: usize) -> Result<u64, PackingError> {
    if bit_len == 0 {
        return Ok(0);
    }
    if bit_len > 64 {
        return Err(PackingError::InvalidBitLength {
            bit_len,
            max_allowed: 64,
        });
    }

    let total_bits = buf.len() * 8;
    let end_bit = bit_offset
        .checked_add(bit_len)
        .ok_or(PackingError::OutOfBounds {
            bit_offset,
            bit_len,
            capacity_bits: total_bits,
        })?;

    if end_bit > total_bits {
        return Err(PackingError::OutOfBounds {
            bit_offset,
            bit_len,
            capacity_bits: total_bits,
        });
    }

    let start_byte = bit_offset / 8;
    let bit_shift = (bit_offset % 8) as u32;
    let num_bytes = ((bit_shift as usize + bit_len + 7) / 8).min(buf.len() - start_byte);

    let mut accumulator: u128 = 0;
    for i in 0..num_bytes {
        accumulator |= (buf[start_byte + i] as u128) << (i * 8);
    }

    let mask = if bit_len == 64 {
        u64::MAX
    } else {
        (1u64 << bit_len) - 1
    };

    let result = ((accumulator >> bit_shift) as u64) & mask;
    Ok(result)
}

/// Writes an unaligned bit sequence up to 64 bits into a byte buffer at an arbitrary bit offset.
///
/// Bits in the target range are replaced; all surrounding bits in touched bytes are preserved.
///
/// # Arguments
/// * `buf` - Target mutable byte slice.
/// * `bit_offset` - Zero-indexed bit position to begin writing.
/// * `bit_len` - Number of bits to write (must be in range `[0, 64]`).
/// * `value` - Value to write (truncated to `bit_len` least significant bits).
///
/// # Errors
/// Returns [`PackingError::InvalidBitLength`] if `bit_len > 64`, or
/// [`PackingError::OutOfBounds`] if the target range exceeds `buf.len() * 8`.
#[inline]
pub fn write_bits(
    buf: &mut [u8],
    bit_offset: usize,
    bit_len: usize,
    value: u64,
) -> Result<(), PackingError> {
    if bit_len == 0 {
        return Ok(());
    }
    if bit_len > 64 {
        return Err(PackingError::InvalidBitLength {
            bit_len,
            max_allowed: 64,
        });
    }

    let total_bits = buf.len() * 8;
    let end_bit = bit_offset
        .checked_add(bit_len)
        .ok_or(PackingError::OutOfBounds {
            bit_offset,
            bit_len,
            capacity_bits: total_bits,
        })?;

    if end_bit > total_bits {
        return Err(PackingError::OutOfBounds {
            bit_offset,
            bit_len,
            capacity_bits: total_bits,
        });
    }

    let start_byte = bit_offset / 8;
    let bit_shift = (bit_offset % 8) as u32;
    let num_bytes = (bit_shift as usize + bit_len + 7) / 8;

    let mut accumulator: u128 = 0;
    for i in 0..num_bytes {
        accumulator |= (buf[start_byte + i] as u128) << (i * 8);
    }

    let mask = if bit_len == 64 {
        u128::from(u64::MAX)
    } else {
        (1u128 << bit_len) - 1
    };

    let masked_val = (value as u128) & mask;
    let clear_mask = !(mask << bit_shift);
    accumulator = (accumulator & clear_mask) | (masked_val << bit_shift);

    for i in 0..num_bytes {
        buf[start_byte + i] = ((accumulator >> (i * 8)) & 0xFF) as u8;
    }

    Ok(())
}

/// Shifts a range of bits rightward (towards higher bit indices) by `shift_amount` bits.
///
/// Iterates backward from the end of the range to prevent overwriting unread bits.
pub fn shift_bits_right(
    buf: &mut [u8],
    start_bit: usize,
    num_bits: usize,
    shift_amount: usize,
) -> Result<(), PackingError> {
    if num_bits == 0 || shift_amount == 0 {
        return Ok(());
    }

    let total_bits = buf.len() * 8;
    let target_end = start_bit
        .checked_add(num_bits)
        .and_then(|v| v.checked_add(shift_amount))
        .ok_or(PackingError::OutOfBounds {
            bit_offset: start_bit,
            bit_len: num_bits + shift_amount,
            capacity_bits: total_bits,
        })?;

    if target_end > total_bits {
        return Err(PackingError::OutOfBounds {
            bit_offset: start_bit,
            bit_len: num_bits + shift_amount,
            capacity_bits: total_bits,
        });
    }

    let mut remaining = num_bits;
    while remaining > 0 {
        let chunk_len = remaining.min(56);
        let src_offset = start_bit + remaining - chunk_len;
        let dst_offset = src_offset + shift_amount;

        let val = read_bits(buf, src_offset, chunk_len)?;
        write_bits(buf, dst_offset, chunk_len, val)?;

        remaining -= chunk_len;
    }

    Ok(())
}

/// Shifts a range of bits leftward (towards lower bit indices) by `shift_amount` bits.
///
/// Iterates forward from the start of the range to prevent overwriting unread bits.
pub fn shift_bits_left(
    buf: &mut [u8],
    start_bit: usize,
    num_bits: usize,
    shift_amount: usize,
) -> Result<(), PackingError> {
    if num_bits == 0 || shift_amount == 0 {
        return Ok(());
    }

    let total_bits = buf.len() * 8;
    if start_bit < shift_amount {
        return Err(PackingError::OutOfBounds {
            bit_offset: start_bit,
            bit_len: num_bits,
            capacity_bits: total_bits,
        });
    }

    let src_end = start_bit
        .checked_add(num_bits)
        .ok_or(PackingError::OutOfBounds {
            bit_offset: start_bit,
            bit_len: num_bits,
            capacity_bits: total_bits,
        })?;

    if src_end > total_bits {
        return Err(PackingError::OutOfBounds {
            bit_offset: start_bit,
            bit_len: num_bits,
            capacity_bits: total_bits,
        });
    }

    let mut offset = 0;
    while offset < num_bits {
        let chunk_len = (num_bits - offset).min(56);
        let src_offset = start_bit + offset;
        let dst_offset = src_offset - shift_amount;

        let val = read_bits(buf, src_offset, chunk_len)?;
        write_bits(buf, dst_offset, chunk_len, val)?;

        offset += chunk_len;
    }

    Ok(())
}

/// Inserts a variable-length fingerprint into the 48-byte payload, shifting trailing bits right.
///
/// # Arguments
/// * `payload` - Fixed 48-byte block payload array.
/// * `bit_offset` - Bit offset where the new fingerprint is inserted.
/// * `total_bits_used` - Current total bits occupied by existing fingerprints.
/// * `bit_len` - Width of the fingerprint to insert.
/// * `fingerprint` - Value of the fingerprint.
///
/// # Errors
/// Returns [`PackingError::CapacityOverflow`] if `total_bits_used + bit_len > 384`.
pub fn pack_fingerprint(
    payload: &mut [u8; PAYLOAD_SIZE_BYTES],
    bit_offset: usize,
    total_bits_used: usize,
    bit_len: usize,
    fingerprint: u64,
) -> Result<(), PackingError> {
    let required = total_bits_used.checked_add(bit_len).ok_or(
        PackingError::CapacityOverflow {
            required_bits: usize::MAX,
            max_bits: PAYLOAD_CAPACITY_BITS,
        },
    )?;

    if required > PAYLOAD_CAPACITY_BITS {
        return Err(PackingError::CapacityOverflow {
            required_bits: required,
            max_bits: PAYLOAD_CAPACITY_BITS,
        });
    }

    if bit_offset < total_bits_used {
        let shift_len = total_bits_used - bit_offset;
        shift_bits_right(payload, bit_offset, shift_len, bit_len)?;
    }

    write_bits(payload, bit_offset, bit_len, fingerprint)
}

/// Reads a packed fingerprint of length `bit_len` from `bit_offset`.
#[inline]
pub fn unpack_fingerprint(
    payload: &[u8; PAYLOAD_SIZE_BYTES],
    bit_offset: usize,
    bit_len: usize,
) -> Result<u64, PackingError> {
    read_bits(payload, bit_offset, bit_len)
}

/// Removes a fingerprint from the payload and shifts trailing bits left.
///
/// The vacated trailing bits are cleared to zero.
pub fn remove_fingerprint(
    payload: &mut [u8; PAYLOAD_SIZE_BYTES],
    bit_offset: usize,
    total_bits_used: usize,
    bit_len: usize,
) -> Result<(), PackingError> {
    if bit_offset + bit_len > total_bits_used {
        return Err(PackingError::OutOfBounds {
            bit_offset,
            bit_len,
            capacity_bits: total_bits_used,
        });
    }

    let trailing_start = bit_offset + bit_len;
    let trailing_bits = total_bits_used - trailing_start;

    if trailing_bits > 0 {
        shift_bits_left(payload, trailing_start, trailing_bits, bit_len)?;
    }

    // Zero out the vacated tail bits
    write_bits(payload, total_bits_used - bit_len, bit_len, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_write_single_bits() {
        let mut buf = [0u8; 8];

        write_bits(&mut buf, 0, 1, 1).unwrap();
        assert_eq!(read_bits(&buf, 0, 1).unwrap(), 1);
        assert_eq!(read_bits(&buf, 1, 1).unwrap(), 0);

        write_bits(&mut buf, 7, 1, 1).unwrap();
        assert_eq!(read_bits(&buf, 7, 1).unwrap(), 1);
        assert_eq!(buf[0], 0b1000_0001);
    }

    #[test]
    fn test_cross_byte_boundaries() {
        let mut buf = [0u8; 8];
        let val = 0b1101_1010_1111; // 12 bits

        write_bits(&mut buf, 5, 12, val).unwrap();
        let extracted = read_bits(&buf, 5, 12).unwrap();
        assert_eq!(extracted, val);
    }

    #[test]
    fn test_maximum_scalar_width() {
        let mut buf = [0u8; 16];
        let val = 0xFEDC_BA98_7654_3210;

        write_bits(&mut buf, 3, 64, val).unwrap();
        assert_eq!(read_bits(&buf, 3, 64).unwrap(), val);
    }

    #[test]
    fn test_pack_unpack_multiple_fingerprints() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        let fingerprints = [0xA5u64, 0x5Au64, 0x3Cu64, 0xC3u64];
        let bit_len = 8;
        let mut total_bits = 0;

        for (i, &fp) in fingerprints.iter().enumerate() {
            let offset = i * bit_len;
            pack_fingerprint(&mut payload, offset, total_bits, bit_len, fp).unwrap();
            total_bits += bit_len;
        }

        assert_eq!(total_bits, 32);

        for (i, &expected) in fingerprints.iter().enumerate() {
            let offset = i * bit_len;
            let unpacked = unpack_fingerprint(&payload, offset, bit_len).unwrap();
            assert_eq!(unpacked, expected, "Mismatch at index {i}");
        }
    }

    #[test]
    fn test_insert_and_shift_payload() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        let bit_len = 8;

        // Insert FP0 = 0x11, FP1 = 0x33
        pack_fingerprint(&mut payload, 0, 0, bit_len, 0x11).unwrap();
        pack_fingerprint(&mut payload, 8, 8, bit_len, 0x33).unwrap();

        // Insert FP_mid = 0x22 at offset 8 (between FP0 and FP1)
        pack_fingerprint(&mut payload, 8, 16, bit_len, 0x22).unwrap();

        assert_eq!(unpack_fingerprint(&payload, 0, 8).unwrap(), 0x11);
        assert_eq!(unpack_fingerprint(&payload, 8, 8).unwrap(), 0x22);
        assert_eq!(unpack_fingerprint(&payload, 16, 8).unwrap(), 0x33);
    }

    #[test]
    fn test_remove_and_shift_payload() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        let bit_len = 8;

        pack_fingerprint(&mut payload, 0, 0, bit_len, 0x11).unwrap();
        pack_fingerprint(&mut payload, 8, 8, bit_len, 0x22).unwrap();
        pack_fingerprint(&mut payload, 16, 16, bit_len, 0x33).unwrap();

        // Remove element at offset 8 (0x22)
        remove_fingerprint(&mut payload, 8, 24, bit_len).unwrap();

        assert_eq!(unpack_fingerprint(&payload, 0, 8).unwrap(), 0x11);
        assert_eq!(unpack_fingerprint(&payload, 8, 8).unwrap(), 0x33);
        // Vacated tail is zeroed
        assert_eq!(unpack_fingerprint(&payload, 16, 8).unwrap(), 0);
    }

    #[test]
    fn test_capacity_overflow() {
        let mut payload = [0u8; PAYLOAD_SIZE_BYTES];
        let err = pack_fingerprint(
            &mut payload,
            0,
            PAYLOAD_CAPACITY_BITS,
            1,
            1,
        )
            .unwrap_err();

        assert_eq!(
            err,
            PackingError::CapacityOverflow {
                required_bits: PAYLOAD_CAPACITY_BITS + 1,
                max_bits: PAYLOAD_CAPACITY_BITS,
            }
        );
    }
}