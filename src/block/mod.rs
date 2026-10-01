//! 64-byte cacheline block abstractions, bit-packing, and SIMD scanning routines.

pub mod layout;
pub mod packing;
pub mod simd;

pub use layout::{Block, BLOCK_SIZE_BYTES, PAYLOAD_SIZE_BYTES};
pub use packing::{
    fingerprint_len, pack_fingerprint, read_bits, remove_fingerprint, unpack_fingerprint,
    write_bits, PackingError, PAYLOAD_CAPACITY_BITS,
};
pub use simd::{
    find_fingerprint_in_run, rank64, scan_payload_bytes, select64, select64_scalar,
};