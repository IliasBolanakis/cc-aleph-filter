//! C-compatible Foreign Function Interface (FFI) for CC-Aleph.
//!
//! Provides ABI-stable bindings for integration into native C and C++ storage engines
//! such as RocksDB (`rocksdb::FilterPolicy`).

use crate::filter::CcAlephFilter;
use std::slice;

/// Opaque handle representing an allocated [`CcAlephFilter`] instance.
pub type CcAlephHandle = *mut CcAlephFilter;

/// Allocates and initializes a new CC-Aleph filter instance on the heap.
///
/// # Arguments
/// * `base_fp_bits` - Initial fingerprint bitwidth (e.g., 16 to 24 bits).
///
/// # Safety
/// The returned pointer must be freed using [`cc_aleph_destroy`] to prevent memory leaks.
#[no_mangle]
pub extern "C" fn cc_aleph_create(base_fp_bits: usize) -> CcAlephHandle {
    let filter = Box::new(CcAlephFilter::with_fingerprint_bits(base_fp_bits));
    Box::into_raw(filter)
}

/// Inserts a key into the CC-Aleph filter.
///
/// # Safety
/// * `handle` must be a valid, non-null pointer returned by [`cc_aleph_create`].
/// * `key_ptr` must point to a readable buffer of at least `key_len` bytes.
///
/// # Returns
/// * `0` on successful insertion.
/// * `-1` if any pointer is null or if internal insertion fails.
#[no_mangle]
pub unsafe extern "C" fn cc_aleph_insert(
    handle: CcAlephHandle,
    key_ptr: *const u8,
    key_len: usize,
) -> i32 {
    if handle.is_null() || key_ptr.is_null() {
        return -1;
    }

    let filter = &*handle;
    let key = slice::from_raw_parts(key_ptr, key_len);

    match filter.insert(key) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Checks approximate key membership in the filter.
///
/// # Safety
/// * `handle` must be a valid, non-null pointer returned by [`cc_aleph_create`].
/// * `key_ptr` must point to a readable buffer of at least `key_len` bytes.
///
/// # Returns
/// * `1` if the key is probably present (hit).
/// * `0` if the key is definitely absent (true negative), or if input pointers are null.
#[no_mangle]
pub unsafe extern "C" fn cc_aleph_contains(
    handle: CcAlephHandle,
    key_ptr: *const u8,
    key_len: usize,
) -> i32 {
    if handle.is_null() || key_ptr.is_null() {
        return 0;
    }

    let filter = &*handle;
    let key = slice::from_raw_parts(key_ptr, key_len);

    if filter.contains(key) {
        1
    } else {
        0
    }
}

/// Deallocates a CC-Aleph filter instance previously allocated by [`cc_aleph_create`].
///
/// # Safety
/// * `handle` must be a pointer returned by [`cc_aleph_create`].
/// * Must not be called more than once on the same pointer.
#[no_mangle]
pub unsafe extern "C" fn cc_aleph_destroy(handle: CcAlephHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}