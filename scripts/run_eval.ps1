<#
.SYNOPSIS
    CC-Aleph Comprehensive Performance Evaluation Suite
.DESCRIPTION
    Executes Criterion microbenchmarks, tail latency profiling,
    concurrent throughput benchmarks, and the LSM MemTable simulation.
#>

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " CC-Aleph Empirical Performance Evaluation Suite" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# Ensure native SIMD optimization flags are set
$env:RUSTFLAGS = "-C target-cpu=native"

Write-Host "`n[1/4] Running Intra-Block Vector Microbenchmarks (Criterion)..." -ForegroundColor Yellow
cargo bench --bench bench_micro

Write-Host "`n[2/4] Profiling Ingestion Tail Latency Under Successive Doublings..." -ForegroundColor Yellow
cargo bench --bench bench_latency_tail -- --nocapture

Write-Host "`n[3/4] Benchmarking Multithreaded Ingestion Throughput..." -ForegroundColor Yellow
cargo bench --bench bench_throughput -- --nocapture

Write-Host "`n[4/4] Executing LSM MemTable Write-Buffer Workload..." -ForegroundColor Yellow
cargo run --example memtable_ingest --release

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host " All benchmarks executed successfully." -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green