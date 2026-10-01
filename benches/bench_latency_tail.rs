//! Ingestion tail latency benchmark measuring p50, p99, and p99.9 spikes across doubling events.

use cc_aleph::CcAlephFilter;
use std::time::Instant;

fn main() {
    println!("===============================================================================");
    println!(" CC-Aleph Ingestion Tail Latency & Zero-Stall Verification Harness");
    println!("===============================================================================");

    let filter = CcAlephFilter::with_fingerprint_bits(20);
    let total_inserts = 25_000;
    let mut latencies_ns = Vec::with_capacity(total_inserts);

    println!("[*] Ingesting {total_inserts} keys across multiple directory doublings...");

    let start_total = Instant::now();
    for i in 0..total_inserts {
        let key = format!("latency_sample_key_{i}");
        let key_bytes = key.as_bytes();

        let t0 = Instant::now();
        filter.insert(key_bytes).unwrap();
        let elapsed = t0.elapsed().as_nanos() as u64;

        latencies_ns.push(elapsed);
    }
    let total_duration = start_total.elapsed();

    latencies_ns.sort_unstable();

    let p50 = latencies_ns[(total_inserts as f64 * 0.50) as usize];
    let p90 = latencies_ns[(total_inserts as f64 * 0.90) as usize];
    let p99 = latencies_ns[(total_inserts as f64 * 0.99) as usize];
    let p99_9 = latencies_ns[(total_inserts as f64 * 0.999) as usize];
    let max = latencies_ns[total_inserts - 1];

    let throughput_mops = (total_inserts as f64 / total_duration.as_secs_f64()) / 1_000_000.0;

    println!("-------------------------------------------------------------------------------");
    println!(" Results:");
    println!("  Total Keys Inserted:   {total_inserts}");
    println!("  Allocated Blocks:      {}", filter.num_blocks());
    println!("  Directory Depth:       {}", filter.global_depth());
    println!("  Overall Throughput:    {throughput_mops:.2} Mops/sec");
    println!("-------------------------------------------------------------------------------");
    println!(" Ingestion Latency Distribution (nanoseconds):");
    println!(
        "  p50 (Median):          {p50} ns ({:.2} µs)",
        p50 as f64 / 1_000.0
    );
    println!(
        "  p90:                   {p90} ns ({:.2} µs)",
        p90 as f64 / 1_000.0
    );
    println!(
        "  p99:                   {p99} ns ({:.2} µs)",
        p99 as f64 / 1_000.0
    );
    println!(
        "  p99.9:                 {p99_9} ns ({:.2} µs)",
        p99_9 as f64 / 1_000.0
    );
    println!(
        "  Max:                   {max} ns ({:.2} µs)",
        max as f64 / 1_000.0
    );
    println!("===============================================================================");
}
