//! In-Memory LSM-Tree MemTable shim integrated with CC-Aleph AMQ filter.
//!
//! Demonstrates zero-stall ingestion and negative lookup short-circuiting
//! under a concurrent read/write database workload.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use cc_aleph::CcAlephFilter;
use parking_lot::RwLock;

/// Concurrent LSM Write Buffer (MemTable) equipped with an active CC-Aleph filter.
pub struct LsmMemTable {
    /// SkipList / BTreeMap representation of the active in-memory table.
    table: RwLock<BTreeMap<Vec<u8>, Vec<u8>>>,
    /// Cache-partitioned AMQ filter guarding against unnecessary MemTable scans.
    filter: CcAlephFilter,
    /// Counter tracking disk / memory lookups averted due to negative filter checks.
    bypassed_lookups: AtomicUsize,
    /// Counter tracking exact-match lookups serviced.
    hit_lookups: AtomicUsize,
}

impl LsmMemTable {
    /// Creates a new MemTable with an unbounded dynamic CC-Aleph filter.
    pub fn new(base_fp_bits: usize) -> Self {
        Self {
            table: RwLock::new(BTreeMap::new()),
            filter: CcAlephFilter::with_fingerprint_bits(base_fp_bits),
            bypassed_lookups: AtomicUsize::new(0),
            hit_lookups: AtomicUsize::new(0),
        }
    }

    /// Appends a key-value pair to the write buffer and registers it in the filter.
    pub fn put(&self, key: &[u8], value: &[u8]) {
        // 1. Ingest into the filter first (zero-stall dynamic expansion)
        self.filter.insert(key).expect("Filter capacity overflow");

        // 2. Insert into the sorted memory table
        let mut guard = self.table.write();
        guard.insert(key.to_vec(), value.to_vec());
    }

    /// Queries the MemTable for a target key.
    ///
    /// If CC-Aleph returns `false`, the expensive table lock and search are completely bypassed.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        // Fast-path AMQ check
        if !self.filter.contains(key) {
            self.bypassed_lookups.fetch_add(1, Ordering::Relaxed);
            return None; // 100% true negative guarantee
        }

        // Slow-path table inspection
        let guard = self.table.read();
        let result = guard.get(key).cloned();
        if result.is_some() {
            self.hit_lookups.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Returns the number of avoided searches.
    pub fn bypassed_count(&self) -> usize {
        self.bypassed_lookups.load(Ordering::Relaxed)
    }

    /// Returns the number of successful positive queries.
    pub fn hit_count(&self) -> usize {
        self.hit_lookups.load(Ordering::Relaxed)
    }
}

fn main() {
    println!("===============================================================================");
    println!(" CC-Aleph Storage Engine Integration: LSM MemTable Ingestion Simulation");
    println!("===============================================================================");

    let memtable = Arc::new(LsmMemTable::new(20));
    let num_writer_threads = 4;
    let items_per_writer = 10_000;
    let num_reader_threads = 4;

    println!(
        "[*] Launching {} writers ({} keys each) and {} concurrent readers...",
        num_writer_threads, items_per_writer, num_reader_threads
    );

    let start = Instant::now();
    let mut handles = Vec::new();

    // 1. Writer threads generating sequential writes
    for w_id in 0..num_writer_threads {
        let mt = Arc::clone(&memtable);
        let handle = thread::spawn(move || {
            for i in 0..items_per_writer {
                let key = format!("user_{:08}_record_{}", w_id, i);
                let val = format!("payload_data_blob_{}", i);
                mt.put(key.as_bytes(), val.as_bytes());
            }
        });
        handles.push(handle);
    }

    // 2. Reader threads issuing mixed queries (90% non-existent keys, 10% valid keys)
    for r_id in 0..num_reader_threads {
        let mt = Arc::clone(&memtable);
        let handle = thread::spawn(move || {
            for i in 0..10_000 {
                if i % 10 == 0 {
                    // Positive query probe
                    let key = format!("user_{:08}_record_{}", r_id % num_writer_threads, i % 100);
                    let _ = mt.get(key.as_bytes());
                } else {
                    // Negative query probe
                    let missing = format!("ghost_key_{:08}_{}", r_id, i);
                    let _ = mt.get(missing.as_bytes());
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let elapsed = start.elapsed();
    let total_records = num_writer_threads * items_per_writer;
    let total_throughput =
        ((total_records + (num_reader_threads * 10_000)) as f64 / elapsed.as_secs_f64()) / 1_000_000.0;

    println!("-------------------------------------------------------------------------------");
    println!(" Simulation Execution Summary:");
    println!("  Total Records Ingested:    {}", total_records);
    println!("  Execution Elapsed Time:    {:.2?}", elapsed);
    println!("  Blended System Throughput: {:.2} Mops/sec", total_throughput);
    println!("  Negative Lookups Bypassed: {}", memtable.bypassed_count());
    println!("  Positive Lookups Serviced: {}", memtable.hit_count());
    println!("  Filter Active Blocks:      {}", memtable.filter.num_blocks());
    println!("  Filter Global Depth:       {}", memtable.filter.global_depth());
    println!("===============================================================================");
}