//! Optimistic sequence lock (seqlock) for wait-free concurrent readers.
//!
//! Provides lock-free optimistic versioning for readers during block lookups
//! and mutually exclusive write access during insertions and migrations.

use core::hint::spin_loop;
use core::sync::atomic::{fence, AtomicU64, Ordering};
use parking_lot::{Mutex, MutexGuard};

/// Optimistic sequence lock protecting 64-byte block state.
#[derive(Debug, Default)]
pub struct SeqLock {
    /// Monotonically increasing sequence version counter.
    /// Even numbers denote clean state; odd numbers denote an in-flight write.
    sequence: AtomicU64,
    /// Mutex serializing concurrent writers on the same block.
    write_mutex: Mutex<()>,
}

/// RAII guard representing exclusive write access to a protected block.
pub struct SeqLockWriteGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    seqlock: &'a SeqLock,
}

impl SeqLock {
    /// Creates a new sequence lock initialized with version 0.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            write_mutex: Mutex::new(()),
        }
    }

    /// Begins an optimistic read transaction.
    ///
    /// Spins if a writer is currently modifying the block until the writer completes.
    /// Returns the observed even sequence version.
    #[inline]
    pub fn read_begin(&self) -> u64 {
        loop {
            let seq = self.sequence.load(Ordering::Acquire);
            if seq & 1 == 0 {
                return seq;
            }
            spin_loop();
        }
    }

    /// Validates whether the data read during the transaction is consistent.
    ///
    /// # Arguments
    /// * `version` - The sequence version returned by [`Self::read_begin`].
    ///
    /// Returns `true` if no writer intervened; `false` if a writer modified the block.
    #[inline]
    #[must_use]
    pub fn read_validate(&self, version: u64) -> bool {
        fence(Ordering::Acquire);
        self.sequence.load(Ordering::Relaxed) == version
    }

    /// Acquires exclusive write access to the block.
    ///
    /// Advances the sequence counter to an odd value, notifying concurrent readers
    /// to invalidate in-flight optimistic reads.
    #[inline]
    pub fn write_lock(&self) -> SeqLockWriteGuard<'_> {
        let guard = self.write_mutex.lock();
        self.sequence.fetch_add(1, Ordering::Release);
        fence(Ordering::Release);
        SeqLockWriteGuard {
            _guard: guard,
            seqlock: self,
        }
    }

    /// Returns the current raw sequence version.
    #[inline]
    pub fn current_version(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }
}

impl<'a> Drop for SeqLockWriteGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        // Advance sequence from odd back to even, marking write completion
        self.seqlock.sequence.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_seqlock_single_thread_lifecycle() {
        let lock = SeqLock::new();
        assert_eq!(lock.current_version(), 0);

        let v1 = lock.read_begin();
        assert_eq!(v1, 0);
        assert!(lock.read_validate(v1));

        {
            let _write = lock.write_lock();
            assert_eq!(lock.current_version(), 1);
            // Reader must fail validation during an in-flight write
            assert!(!lock.read_validate(v1));
        }

        assert_eq!(lock.current_version(), 2);
        let v2 = lock.read_begin();
        assert_eq!(v2, 2);
        assert!(lock.read_validate(v2));
        assert!(!lock.read_validate(v1));
    }

    #[test]
    fn test_seqlock_concurrent_readers_writers() {
        let lock = Arc::new(SeqLock::new());
        let shared_data = Arc::new(AtomicU64::new(0));

        let lock_writer = Arc::clone(&lock);
        let data_writer = Arc::clone(&shared_data);

        let writer_handle = thread::spawn(move || {
            for i in 1..=500 {
                let _guard = lock_writer.write_lock();
                data_writer.store(i, Ordering::Relaxed);
            }
        });

        let lock_reader = Arc::clone(&lock);
        let data_reader = Arc::clone(&shared_data);

        let reader_handle = thread::spawn(move || {
            let mut successful_reads = 0;
            while successful_reads < 500 {
                let v = lock_reader.read_begin();
                let val = data_reader.load(Ordering::Relaxed);
                if lock_reader.read_validate(v) {
                    successful_reads += 1;
                    assert!(val <= 500);
                }
            }
        });

        writer_handle.join().unwrap();
        reader_handle.join().unwrap();
    }
}
