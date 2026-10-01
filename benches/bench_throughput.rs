//! Multithreaded scaling benchmarks measuring operations per second.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use cc_aleph::CcAlephFilter;

fn bench_concurrent_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("ConcurrentReadThroughput");

    let filter = Arc::new(CcAlephFilter::with_fingerprint_bits(18));
    for i in 0..20_000 {
        let key = format!("seed_key_{i}");
        filter.insert(key.as_bytes()).unwrap();
    }

    for threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{threads}_threads")),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let stop = Arc::new(AtomicBool::new(false));
                    let mut handles = Vec::new();

                    for t in 0..threads {
                        let f = Arc::clone(&filter);
                        let s = Arc::clone(&stop);
                        let handle = thread::spawn(move || {
                            let mut ops = 0usize;
                            for i in 0..1_000 {
                                let key = format!("seed_key_{}", (t * 1_000) + i);
                                if black_box(f.contains(black_box(key.as_bytes()))) {
                                    ops += 1;
                                }
                            }
                            ops
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        let _ = handle.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_concurrent_reads);
criterion_main!(benches);