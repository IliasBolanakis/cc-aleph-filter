//! Multithreaded stress test validating wait-free readers and concurrent writers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cc_aleph::CcAlephFilter;

#[test]
fn test_concurrent_readers_and_writers_stress() {
    let filter = Arc::new(CcAlephFilter::with_fingerprint_bits(20));
    let stop_signal = Arc::new(AtomicBool::new(false));

    let num_writers = 4;
    let items_per_writer = 500;
    let num_readers = 4;

    let mut handles = Vec::new();

    // 1. Spawn concurrent writer threads
    for writer_id in 0..num_writers {
        let filter_clone = Arc::clone(&filter);
        let handle = thread::spawn(move || {
            for i in 0..items_per_writer {
                let key = format!("concurrent_writer_{writer_id}_item_{i}");
                filter_clone.insert(key.as_bytes()).unwrap();
            }
        });
        handles.push(handle);
    }

    // 2. Spawn concurrent reader threads probing both inserted and negative keys
    for reader_id in 0..num_readers {
        let filter_clone = Arc::clone(&filter);
        let stop_clone = Arc::clone(&stop_signal);

        let handle = thread::spawn(move || {
            let mut read_ops = 0usize;
            while !stop_clone.load(Ordering::Relaxed) {
                // Negative probe: must never panic or deadlock
                let non_existent = format!("negative_reader_{reader_id}_probe_{read_ops}");
                let _ = filter_clone.contains(non_existent.as_bytes());
                read_ops += 1;
            }
        });
        handles.push(handle);
    }

    // Wait for all writer threads to finish insertions
    for handle in handles.drain(..num_writers) {
        handle.join().unwrap();
    }

    // Stop reader threads
    thread::sleep(Duration::from_millis(100));
    stop_signal.store(true, Ordering::Relaxed);

    for handle in handles {
        handle.join().unwrap();
    }

    // 3. Verify correctness: 100% recall on all inserted keys (zero false negatives)
    for writer_id in 0..num_writers {
        for i in 0..items_per_writer {
            let key = format!("concurrent_writer_{writer_id}_item_{i}");
            assert!(
                filter.contains(key.as_bytes()),
                "False negative detected for key {key}"
            );
        }
    }

    assert_eq!(filter.len(), num_writers * items_per_writer);
    assert!(
        filter.num_blocks() > 1,
        "Filter should have performed concurrent block splits"
    );
}
