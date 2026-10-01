#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================="
echo " CC-Aleph Empirical Performance Evaluation Suite"
echo "=========================================================="

export RUSTFLAGS="-C target-cpu=native"

echo ""
echo "[1/4] Running Intra-Block Vector Microbenchmarks (Criterion)..."
cargo bench --bench bench_micro

echo ""
echo "[2/4] Profiling Ingestion Tail Latency Under Successive Doublings..."
cargo bench --bench bench_latency_tail -- --nocapture

echo ""
echo "[3/4] Benchmarking Multithreaded Ingestion Throughput..."
cargo bench --bench bench_throughput -- --nocapture

echo ""
echo "[4/4] Executing LSM MemTable Write-Buffer Workload..."
cargo run --example memtable_ingest --release

echo ""
echo "=========================================================="
echo " All benchmarks executed successfully."
echo "=========================================================="