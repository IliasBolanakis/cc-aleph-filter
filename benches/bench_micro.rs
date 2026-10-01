//! Microbenchmarks measuring intra-block SIMD efficiency and point lookups.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cc_aleph::block::layout::Block;
use cc_aleph::block::simd::{scan_payload_bytes, scan_payload_scalar};
use cc_aleph::CcAlephFilter;

fn bench_simd_vs_scalar_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("IntraBlockScan48Bytes");

    let mut payload = [0u8; 48];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = (i * 7) as u8;
    }
    let target = payload[35];

    group.bench_function("ScalarScan", |b| {
        b.iter(|| {
            black_box(scan_payload_scalar(black_box(&payload), black_box(target)));
        });
    });

    group.bench_function("AVX2Scan", |b| {
        b.iter(|| {
            black_box(scan_payload_bytes(black_box(&payload), black_box(target)));
        });
    });

    group.finish();
}

fn bench_block_insert_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("BlockOperations");

    group.bench_function("BlockInsertSingle", |b| {
        b.iter_batched(
            Block::new,
            |mut block| {
                let res = block.insert(black_box(7), black_box(0xAB), black_box(8));
                black_box(res).unwrap();
            },
            criterion::BatchSize::SmallInput,
        );
    });

    let mut populated_block = Block::new();
    for i in 0..16 {
        populated_block.insert(i, (i as u64) + 0x10, 8).unwrap();
    }

    group.bench_function("BlockQueryHit", |b| {
        b.iter(|| {
            black_box(populated_block.query(black_box(7), black_box(0x17), black_box(8)));
        });
    });

    group.bench_function("BlockQueryMiss", |b| {
        b.iter(|| {
            black_box(populated_block.query(black_box(7), black_box(0xFF), black_box(8)));
        });
    });

    group.finish();
}

fn bench_filter_point_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("FilterLookups");
    let filter = CcAlephFilter::with_fingerprint_bits(18);

    let num_items = 10_000;
    for i in 0..num_items {
        let key = format!("bench_key_{i}");
        filter.insert(key.as_bytes()).unwrap();
    }

    group.bench_function("LookupExistingKey", |b| {
        let mut idx = 0usize;
        b.iter(|| {
            let key = format!("bench_key_{idx}");
            black_box(filter.contains(black_box(key.as_bytes())));
            idx = (idx + 1) % num_items;
        });
    });

    group.bench_function("LookupNonExistentKey", |b| {
        let mut idx = 0usize;
        b.iter(|| {
            let key = format!("missing_key_{idx}");
            black_box(filter.contains(black_box(key.as_bytes())));
            idx += 1;
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simd_vs_scalar_scan,
    bench_block_insert_query,
    bench_filter_point_lookup
);
criterion_main!(benches);
