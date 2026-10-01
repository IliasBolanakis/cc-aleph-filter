//! Two-level extendible directory and buddy-block migration protocols.
//!
//! Maps logical hashes to physical 64-byte blocks, manages dynamic table doubling,
//! and orchestrates block splitting using fingerprint bit-sacrifice.

pub mod buddy;
pub mod seqlock;

use core::cell::UnsafeCell;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::collections::HashSet;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::{spawn, JoinHandle};

use crossbeam_epoch::{pin, Atomic, Guard, Owned, Shared};
use parking_lot::Mutex;
use xxhash_rust::xxh3::xxh3_128;

use crate::block::layout::Block;
use crate::block::packing::{fingerprint_len, PackingError};
use crate::hash::{decompose_hash, DEFAULT_BASE_FP_BITS, INTRA_BLOCK_QUOTIENT_BITS};

pub use buddy::split_block;
pub use seqlock::{SeqLock, SeqLockWriteGuard};

/// Default maximum load factor threshold ($\alpha = 0.85$) triggering an asynchronous split.
pub const DEFAULT_MAX_LOAD_FACTOR: f64 = 0.85;

/// Capacity of the background expansion worker channel queue.
pub const BACKGROUND_QUEUE_CAPACITY: usize = 4096;

/// Hard capacity ceiling before a writer must perform a synchronous fallback split.
pub const HARD_BLOCK_CAPACITY_LIMIT: usize = Block::SLOTS_PER_BLOCK - 1;

/// Errors that can occur during directory operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryError {
    /// Directory cannot double further because fingerprint length has reached its minimum bound.
    MaxDepthReached {
        /// Current local depth of the saturated block.
        current_depth: usize,
        /// Minimum permissible fingerprint width.
        min_fp_bits: usize,
    },
    /// Low-level bit packing or capacity error within a block.
    Packing(PackingError),
}

impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MaxDepthReached {
                current_depth,
                min_fp_bits,
            } => {
                write!(
                    f,
                    "Maximum directory depth reached at depth {current_depth}: fingerprint length cannot shrink below {min_fp_bits} bits"
                )
            }
            Self::Packing(ref err) => write!(f, "Block packing error: {err}"),
        }
    }
}

impl std::error::Error for DirectoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Packing(ref err) => Some(err),
            Self::MaxDepthReached { .. } => None,
        }
    }
}

impl From<PackingError> for DirectoryError {
    #[inline]
    fn from(err: PackingError) -> Self {
        Self::Packing(err)
    }
}

/// Commands processed by the dedicated background expansion worker.
enum ExpansionJob {
    /// Request to asynchronously split a saturated block node.
    Split(usize),
    /// Clean shutdown command for worker termination.
    Terminate,
}

/// Heap-allocated node holding a 64-byte block, its sequence lock, and local depth.
pub struct BlockNode {
    /// Sequence lock protecting block state during writes and migrations[cite: 1].
    pub seqlock: SeqLock,
    /// Interior mutable 64-byte block[cite: 1].
    pub block: UnsafeCell<Block>,
    /// Local depth $d_i$ of this block.
    pub local_depth: AtomicUsize,
    /// Atomic flag preventing duplicate background split requests for the same block.
    pub is_splitting: AtomicBool,
}

// Safety: Access to `block` is synchronized via `seqlock`.
unsafe impl Send for BlockNode {}
unsafe impl Sync for BlockNode {}

impl BlockNode {
    /// Creates a new block node with the given initial depth and block data.
    #[must_use]
    pub fn new(local_depth: usize, block: Block) -> Self {
        Self {
            seqlock: SeqLock::new(),
            block: UnsafeCell::new(block),
            local_depth: AtomicUsize::new(local_depth),
            is_splitting: AtomicBool::new(false),
        }
    }
}

/// Snapshot of the directory pointer table mapping slots to block nodes[cite: 1].
pub struct DirectoryTable {
    /// Global directory depth $D$, indexing $2^D$ slots.
    pub global_depth: usize,
    /// Pointers to physical block nodes.
    pub slots: Vec<AtomicPtr<BlockNode>>,
}

// Safety: BlockNode pointers within slots are reclaimed via Epoch-Based Memory Reclamation[cite: 1].
unsafe impl Send for DirectoryTable {}
unsafe impl Sync for DirectoryTable {}

impl DirectoryTable {
    /// Creates a new directory table of capacity $2^{\text{global\_depth}}$ pointing to `initial_node`.
    #[must_use]
    pub fn new(global_depth: usize, initial_node: *mut BlockNode) -> Self {
        let size = 1usize << global_depth;
        let mut slots = Vec::with_capacity(size);
        for _ in 0..size {
            slots.push(AtomicPtr::new(initial_node));
        }
        Self {
            global_depth,
            slots,
        }
    }
}

/// Inner state of the concurrent directory shared between front-end callers and background workers.
struct DirectoryInner {
    /// Atomic pointer to the active directory table.
    table: Atomic<DirectoryTable>,
    /// Base fingerprint width at depth 0.
    base_fp_bits: usize,
    /// Element count threshold in a single block triggering an asynchronous background split.
    split_threshold: usize,
    /// Mutex serializing table doubling and block allocations.
    expansion_lock: Mutex<()>,
    /// Channel sender transmitting expansion tasks to the background worker thread[cite: 1].
    job_sender: SyncSender<ExpansionJob>,
    /// Total count of elements stored across all blocks.
    total_items: AtomicUsize,
    /// Total count of physical blocks allocated.
    total_nodes: AtomicUsize,
}

impl DirectoryInner {
    /// Queries whether `key` is present using wait-free optimistic reads[cite: 1].
    fn query(&self, key: &[u8]) -> bool {
        let hash = xxh3_128(key);
        let guard = &pin();

        loop {
            let table_shared = self.table.load(Ordering::Acquire, guard);
            let table = match unsafe { table_shared.as_ref() } {
                Some(t) => t,
                None => return false,
            };

            let global_d = table.global_depth;
            let dir_idx = if global_d == 0 {
                0
            } else {
                let rest = hash >> INTRA_BLOCK_QUOTIENT_BITS;
                (rest as usize) & ((1usize << global_d) - 1)
            };

            let node_ptr = table.slots[dir_idx].load(Ordering::Acquire);
            let node = unsafe { &*node_ptr };

            let version = node.seqlock.read_begin();
            let local_d = node.local_depth.load(Ordering::Acquire);
            let decomp = decompose_hash(hash, local_d, self.base_fp_bits);

            let found = unsafe {
                (*node.block.get()).query(
                    decomp.intra_quotient,
                    decomp.fingerprint,
                    decomp.fp_bit_len,
                )
            };

            if node.seqlock.read_validate(version) {
                if found {
                    return true;
                }

                let cur_table = self.table.load(Ordering::Acquire, guard);
                if cur_table.as_raw() == table_shared.as_raw() {
                    let cur_node_ptr = table.slots[dir_idx].load(Ordering::Acquire);
                    if cur_node_ptr == node_ptr {
                        return false;
                    }
                }
            }
        }
    }

    /// Inserts `key` into the directory with zero-stall asynchronous expansion signaling[cite: 1].
    fn insert(&self, key: &[u8]) -> Result<(), DirectoryError> {
        let hash = xxh3_128(key);
        let guard = &pin();

        loop {
            let table_shared = self.table.load(Ordering::Acquire, guard);
            let table = unsafe { table_shared.as_ref().expect("Table pointer must be valid") };

            let global_d = table.global_depth;
            let dir_idx = if global_d == 0 {
                0
            } else {
                let rest = hash >> INTRA_BLOCK_QUOTIENT_BITS;
                (rest as usize) & ((1usize << global_d) - 1)
            };

            let node_ptr = table.slots[dir_idx].load(Ordering::Acquire);
            let node = unsafe { &*node_ptr };

            let write_guard = node.seqlock.write_lock();

            // Re-validate against the latest active directory table after acquiring write lock
            let cur_table_shared = self.table.load(Ordering::Acquire, guard);
            let cur_table = unsafe {
                cur_table_shared
                    .as_ref()
                    .expect("Table pointer must be valid")
            };
            let cur_global_d = cur_table.global_depth;
            let cur_dir_idx = if cur_global_d == 0 {
                0
            } else {
                let rest = hash >> INTRA_BLOCK_QUOTIENT_BITS;
                (rest as usize) & ((1usize << cur_global_d) - 1)
            };

            let cur_node_ptr = cur_table.slots[cur_dir_idx].load(Ordering::Acquire);
            if cur_node_ptr != node_ptr {
                drop(write_guard);
                continue;
            }

            let current_local_d = node.local_depth.load(Ordering::Acquire);
            let block = unsafe { &mut *node.block.get() };
            let decomp = decompose_hash(hash, current_local_d, self.base_fp_bits);

            let count = block.count();

            // 1. Asynchronous Expansion Trigger: Signal background worker when crossing threshold
            if count >= self.split_threshold && !node.is_splitting.swap(true, Ordering::AcqRel) {
                let _ = self
                    .job_sender
                    .try_send(ExpansionJob::Split(node_ptr as usize));
            }

            // 2. Zero-Stall Fast Path: Insert directly if block has headroom
            if count < HARD_BLOCK_CAPACITY_LIMIT {
                match block.insert(decomp.intra_quotient, decomp.fingerprint, decomp.fp_bit_len) {
                    Ok(()) => {
                        self.total_items.fetch_add(1, Ordering::Relaxed);
                        drop(write_guard);
                        return Ok(());
                    }
                    Err(PackingError::CapacityOverflow { .. }) => {
                        // Packed bits filled prematurely; fall through to synchronous split
                    }
                    Err(err) => {
                        drop(write_guard);
                        return Err(DirectoryError::Packing(err));
                    }
                }
            }

            // 3. Fallback Synchronous Split: Hard capacity reached before background worker completed
            let _exp_guard = self.expansion_lock.lock();
            self.split_and_expand_node(node, current_local_d, guard)?;
            node.is_splitting.store(false, Ordering::Release);
            drop(write_guard);
        }
    }

    /// Background worker execution routine for asynchronous splits[cite: 1].
    fn handle_background_split(&self, node_ptr: *mut BlockNode, guard: &Guard) {
        if node_ptr.is_null() {
            return;
        }
        let node = unsafe { &*node_ptr };
        let write_guard = node.seqlock.write_lock();
        let block = unsafe { &*node.block.get() };
        if block.count() < self.split_threshold {
            node.is_splitting.store(false, Ordering::Release);
            drop(write_guard);
            return;
        }
        let local_depth = node.local_depth.load(Ordering::Acquire);

        #[cfg(feature = "logging")]
        tracing::debug!(
            target_node = ?node_ptr,
            local_depth = local_depth,
            item_count = block.count(),
            "Starting asynchronous buddy-block split"
        );

        let _exp_guard = self.expansion_lock.lock();
        let _ = self.split_and_expand_node(node, local_depth, guard);
        node.is_splitting.store(false, Ordering::Release);
        drop(write_guard);
    }

    /// Splits a saturated block into two daughter blocks and updates directory routing[cite: 1].
    fn split_and_expand_node(
        &self,
        node: &BlockNode,
        local_depth: usize,
        guard: &Guard,
    ) -> Result<(), DirectoryError> {
        let current_fp_len = fingerprint_len(self.base_fp_bits, local_depth);
        if current_fp_len <= 1 {
            return Err(DirectoryError::MaxDepthReached {
                current_depth: local_depth,
                min_fp_bits: 1,
            });
        }

        let original_block = unsafe { &*node.block.get() };
        let (daughter_0, daughter_1) = split_block(original_block, local_depth, self.base_fp_bits)?;

        unsafe {
            *node.block.get() = daughter_0;
        }
        node.local_depth.store(local_depth + 1, Ordering::Release);

        let daughter_1_node = Box::into_raw(Box::new(BlockNode::new(local_depth + 1, daughter_1)));
        self.total_nodes.fetch_add(1, Ordering::Relaxed);

        let table_shared = self.table.load(Ordering::Acquire, guard);
        let table = unsafe { table_shared.as_ref().unwrap() };

        if local_depth == table.global_depth {
            let old_size = table.slots.len();
            let new_size = old_size * 2;
            let mut new_slots = Vec::with_capacity(new_size);

            for i in 0..old_size {
                new_slots.push(AtomicPtr::new(table.slots[i].load(Ordering::Relaxed)));
            }
            for i in 0..old_size {
                new_slots.push(AtomicPtr::new(table.slots[i].load(Ordering::Relaxed)));
            }

            let buddy_bit = 1usize << local_depth;
            for (idx, slot) in new_slots.iter().enumerate() {
                if std::ptr::eq(slot.load(Ordering::Relaxed), node) && (idx & buddy_bit) != 0 {
                    slot.store(daughter_1_node, Ordering::Relaxed);
                }
            }

            let new_table = DirectoryTable {
                global_depth: table.global_depth + 1,
                slots: new_slots,
            };

            #[cfg(feature = "logging")]
            tracing::info!(
                old_depth = table.global_depth,
                new_depth = table.global_depth + 1,
                new_slot_count = new_size,
                "Global directory doubled"
            );

            let old = self
                .table
                .swap(Owned::new(new_table), Ordering::AcqRel, guard);
            unsafe {
                guard.defer_destroy(old);
            }
        } else {
            let buddy_bit = 1usize << local_depth;
            for (idx, slot) in table.slots.iter().enumerate() {
                if std::ptr::eq(slot.load(Ordering::Relaxed), node) && (idx & buddy_bit) != 0 {
                    slot.store(daughter_1_node, Ordering::Release);
                }
            }
        }

        Ok(())
    }
}

impl Drop for DirectoryInner {
    fn drop(&mut self) {
        let guard = &pin();
        let table_shared = self.table.swap(Shared::null(), Ordering::AcqRel, guard);
        let raw = table_shared.as_raw() as *mut DirectoryTable;
        if !raw.is_null() {
            let table = unsafe { Box::from_raw(raw) };
            let mut unique_nodes = HashSet::new();

            for slot in &table.slots {
                let ptr = slot.load(Ordering::Relaxed);
                if !ptr.is_null() {
                    unique_nodes.insert(ptr as usize);
                }
            }

            for ptr_val in unique_nodes {
                unsafe {
                    drop(Box::from_raw(ptr_val as *mut BlockNode));
                }
            }
        }
    }
}

/// Concurrent extendible directory managing asynchronous block expansions and wait-free lookups[cite: 1].
pub struct Directory {
    inner: Arc<DirectoryInner>,
    job_sender: SyncSender<ExpansionJob>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
}

impl core::fmt::Debug for Directory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Directory")
            .field("global_depth", &self.global_depth())
            .field("base_fp_bits", &self.base_fp_bits())
            .field("num_blocks", &self.num_blocks())
            .field("total_items", &self.len())
            .finish()
    }
}

impl Directory {
    /// Creates a new concurrent directory and launches the dedicated expansion worker thread[cite: 1].
    #[must_use]
    pub fn new(base_fp_bits: usize) -> Self {
        let split_threshold = ((Block::SLOTS_PER_BLOCK as f64) * DEFAULT_MAX_LOAD_FACTOR) as usize;
        let mut initial_block = Block::new();
        initial_block.set_generation(0);

        let initial_node = Box::into_raw(Box::new(BlockNode::new(0, initial_block)));
        let table = DirectoryTable::new(0, initial_node);

        let (tx, rx): (SyncSender<ExpansionJob>, Receiver<ExpansionJob>) =
            sync_channel(BACKGROUND_QUEUE_CAPACITY);

        let inner = Arc::new(DirectoryInner {
            table: Atomic::new(table),
            base_fp_bits,
            split_threshold: split_threshold.max(1),
            expansion_lock: Mutex::new(()),
            job_sender: tx.clone(),
            total_items: AtomicUsize::new(0),
            total_nodes: AtomicUsize::new(1),
        });

        let worker_inner = Arc::clone(&inner);
        let worker_handle = spawn(move || {
            #[cfg(feature = "logging")]
            tracing::info!("CC-Aleph expansion worker thread spawned");

            while let Ok(job) = rx.recv() {
                match job {
                    ExpansionJob::Split(node_ptr_val) => {
                        let guard = &pin();
                        worker_inner.handle_background_split(node_ptr_val as *mut BlockNode, guard);
                    }
                    ExpansionJob::Terminate => {
                        #[cfg(feature = "logging")]
                        tracing::info!("CC-Aleph expansion worker received termination signal");
                        break;
                    }
                }
            }
        });

        Self {
            inner,
            job_sender: tx,
            worker_handle: Mutex::new(Some(worker_handle)),
        }
    }

    /// Returns the current global depth $D$ of the directory.
    #[must_use]
    pub fn global_depth(&self) -> usize {
        let guard = &pin();
        let table_shared = self.inner.table.load(Ordering::Acquire, guard);
        unsafe { table_shared.as_ref().map_or(0, |t| t.global_depth) }
    }

    /// Returns the initial base fingerprint bit width.
    #[inline]
    #[must_use]
    pub fn base_fp_bits(&self) -> usize {
        self.inner.base_fp_bits
    }

    /// Returns the number of physical 64-byte blocks allocated.
    #[inline]
    #[must_use]
    pub fn num_blocks(&self) -> usize {
        self.inner.total_nodes.load(Ordering::Relaxed)
    }

    /// Returns the total number of items stored in the filter.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.total_items.load(Ordering::Relaxed)
    }

    /// Returns whether the directory contains zero items.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total slot capacity across all currently allocated blocks.
    #[inline]
    #[must_use]
    pub fn total_slot_capacity(&self) -> usize {
        self.num_blocks() * Block::SLOTS_PER_BLOCK
    }

    /// Queries whether `key` is present in the directory using wait-free optimistic reads[cite: 1].
    #[inline]
    #[must_use]
    pub fn query(&self, key: &[u8]) -> bool {
        self.inner.query(key)
    }

    /// Inserts `key` into the directory, executing asynchronous zero-stall expansions[cite: 1].
    #[inline]
    pub fn insert(&self, key: &[u8]) -> Result<(), DirectoryError> {
        self.inner.insert(key)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = self.job_sender.send(ExpansionJob::Terminate);
        if let Some(handle) = self.worker_handle.lock().take() {
            let _ = handle.join();
        }
    }
}

impl Default for Directory {
    #[inline]
    fn default() -> Self {
        Self::new(DEFAULT_BASE_FP_BITS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_directory_initial_state() {
        let dir = Directory::new(16);
        assert_eq!(dir.global_depth(), 0);
        assert_eq!(dir.num_blocks(), 1);
        assert_eq!(dir.len(), 0);
        assert!(dir.is_empty());
    }

    #[test]
    fn test_directory_insert_and_query_single_block() {
        let dir = Directory::new(16);
        let key1 = b"test_key_1";
        let key2 = b"test_key_2";

        assert!(!dir.query(key1));
        dir.insert(key1).unwrap();
        assert!(dir.query(key1));
        assert!(!dir.query(key2));

        dir.insert(key2).unwrap();
        assert!(dir.query(key1));
        assert!(dir.query(key2));
        assert_eq!(dir.len(), 2);
    }

    #[test]
    fn test_directory_split_and_doubling() {
        let dir = Directory::new(16);
        let num_keys = 200;

        for i in 0..num_keys {
            let key = format!("doubling_key_{i}");
            dir.insert(key.as_bytes()).unwrap();
        }

        assert_eq!(dir.len(), num_keys);
        assert!(dir.global_depth() > 0);
        assert!(dir.num_blocks() > 1);

        for i in 0..num_keys {
            let key = format!("doubling_key_{i}");
            assert!(
                dir.query(key.as_bytes()),
                "Lookup failed for key {key} after expansions"
            );
        }
    }
}
