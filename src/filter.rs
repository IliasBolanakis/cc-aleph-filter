//! Primary filter API and public interface.

use std::sync::Arc;

use crate::directory::{Directory, DirectoryError};
use crate::hash::DEFAULT_BASE_FP_BITS;

/// Cache-Partitioned Concurrent Aleph Filter for high-throughput ingestion[cite: 1].
///
/// Thread-safe and internally synchronized. Can be cloned cheaply and shared
/// across reader and writer threads[cite: 6].
#[derive(Clone)]
pub struct CcAlephFilter {
    /// Internal concurrent extendible directory managing 64-byte blocks[cite: 1, 6].
    directory: Arc<Directory>,
}

impl core::fmt::Debug for CcAlephFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CcAlephFilter")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .field("global_depth", &self.global_depth())
            .field("num_blocks", &self.num_blocks())
            .field("load_factor", &self.load_factor())
            .finish()
    }
}

impl CcAlephFilter {
    /// Creates a new CC-Aleph filter with default configuration parameters[cite: 6].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            directory: Arc::new(Directory::new(DEFAULT_BASE_FP_BITS)),
        }
    }

    /// Creates a new CC-Aleph filter with a specified base fingerprint length[cite: 6].
    ///
    /// # Arguments
    /// * `base_fp_bits` - Initial fingerprint bit length at depth 0[cite: 6].
    #[inline]
    #[must_use]
    pub fn with_fingerprint_bits(base_fp_bits: usize) -> Self {
        Self {
            directory: Arc::new(Directory::new(base_fp_bits)),
        }
    }

    /// Inserts a key into the filter, expanding capacity dynamically as needed[cite: 1, 6].
    ///
    /// Thread-safe and wait-free for concurrent readers[cite: 1, 6].
    ///
    /// # Arguments
    /// * `key` - Arbitrary byte slice[cite: 6].
    ///
    /// # Errors
    /// Returns [`DirectoryError`] if capacity bounds are exceeded[cite: 6].
    #[inline]
    pub fn insert(&self, key: &[u8]) -> Result<(), DirectoryError> {
        self.directory.insert(key)
    }

    /// Queries whether `key` is present in the filter using wait-free optimistic reads[cite: 1, 6].
    ///
    /// Returns `false` with 100% certainty if the key was never inserted[cite: 1, 6].
    /// Returns `true` if the key is likely present (subject to False Positive Rate bounds)[cite: 1, 6].
    #[inline]
    #[must_use]
    pub fn contains(&self, key: &[u8]) -> bool {
        self.directory.query(key)
    }

    /// Returns the total number of items currently inserted into the filter[cite: 6].
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.directory.len()
    }

    /// Returns whether the filter is empty[cite: 6].
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.directory.is_empty()
    }

    /// Returns the current global depth of the underlying directory[cite: 6].
    #[inline]
    #[must_use]
    pub fn global_depth(&self) -> usize {
        self.directory.global_depth()
    }

    /// Returns the number of physical 64-byte cacheline blocks allocated[cite: 6].
    #[inline]
    #[must_use]
    pub fn num_blocks(&self) -> usize {
        self.directory.num_blocks()
    }

    /// Returns the total slot capacity of the filter[cite: 6].
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.directory.total_slot_capacity()
    }

    /// Returns the current load factor ($\alpha$) of the filter[cite: 6].
    #[inline]
    #[must_use]
    pub fn load_factor(&self) -> f64 {
        let cap = self.capacity();
        if cap == 0 {
            0.0
        } else {
            self.len() as f64 / cap as f64
        }
    }
}

impl Default for CcAlephFilter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_public_api() {
        let filter = CcAlephFilter::new();
        assert!(filter.is_empty());
        assert_eq!(filter.len(), 0);

        let key = b"public_api_key";
        assert!(!filter.contains(key));

        filter.insert(key).unwrap();
        assert!(!filter.is_empty());
        assert_eq!(filter.len(), 1);
        assert!(filter.contains(key));
    }

    #[test]
    fn test_filter_debug_format() {
        let filter = CcAlephFilter::new();
        let formatted = format!("{filter:?}");
        assert!(formatted.contains("CcAlephFilter"));
        assert!(formatted.contains("len: 0"));
    }
}
