# CC-Aleph: Cache-Partitioned Concurrent Aleph Filter for Zero-Stall Ingestion in LSM Engines

[![Rust](https://img.shields.io/badge/rust-nightly%20%7C%20stable-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/badge/tests-passing-brightgreen.svg)]()

CC-Aleph is an unbounded, cacheline-aligned Approximate Membership Query (AMQ) filter designed for high-throughput write buffers (MemTables) in Log-Structured Merge (LSM) storage engines[cite: 1, 7]. It addresses the Stop-the-World latency freezes and unaligned memory access patterns inherent in traditional dynamic AMQ filters by combining 64-byte cacheline partitioning, SIMD vectorization, and lock-free extendible directory doubling with Epoch-Based Memory Reclamation (EBMR)[cite: 1, 7].

---

## 1. Architectural Highlights

* **64-Byte Hardware-Aligned Blocks:** Every primary storage unit aligns to an exact CPU cache line (`align(64)`), guaranteeing single-cycle L1-D loads and eliminating cross-cacheline memory stalls[cite: 1, 7].
* **Intra-Block SIMD Vectorization:** Parallel evaluations over the 48-byte fingerprint payload execute via AVX2 (`_mm256_cmpeq_epi8`) and SSE registers, achieving a **$4.10\times$ speedup ($943\text{ ps}$)** over scalar bit traversals[cite: 1, 7].
* **Asynchronous Directory Doubling:** When a block reaches its saturation threshold ($\alpha \ge 0.85$), an asynchronous split is signaled without blocking writer threads, reducing $p99$ tail latency by four orders of magnitude ($1.10\ \mu\text{s}$) compared to Stop-The-World reorganizations[cite: 1, 7].
* **Wait-Free Optimistic Readers:** Readers traverse blocks using optimistic sequence locks (`SeqLock`), remaining wait-free and contention-free even during background buddy-block splits[cite: 1, 7].
* **Safe Memory Lifecycle (EBMR):** Swapped directory pointer tables and retired block nodes are tracked and deferred through `crossbeam-epoch` to eliminate use-after-free races[cite: 1, 7].

---

## 2. Mathematical Model: Fingerprint Bit-Sacrifice

Traditional dynamic filters like InfiniFilter rely on chained overflow structures, degrading lookups to $O(\log N)$ over time[cite: 1, 7]. CC-Aleph maintains strict $O(1)$ lookup complexity by preserving all records within a single two-level directory table through fingerprint bit-sacrifice[cite: 1, 7].

```text
                     64-Byte Cacheline Block
+------------------+------------------+-----------------------------+
| Header & Flags   |  Unary Bitmask   | Packed Fingerprint Payload  |
|    (8 Bytes)     |    (8 Bytes)     |          (48 Bytes)         |
+------------------+------------------+-----------------------------+

```

### 2.1 Intra-Block Quotient Mapping

Each 64-byte block encapsulates $S = 32$ addressable quotient slots. The intra-block quotient $q_{\text{intra}}$ occupies the lowest 5 bits of the 128-bit hash:


$$q_{\text{intra}} = H(x) \pmod{32}$$


Because the intra-block slot count is invariant across splits, $q_{\text{intra}}$ never changes during doubling steps.

### 2.2 Buddy-Block Routing via $b_{\text{MSB}}$

When a block at local depth $d$ reaches capacity, it splits into two daughter blocks at depth $d + 1$. The migration route is determined by the most significant bit ($b_{\text{MSB}}$) of the resident fingerprint:


$$b_{\text{MSB}} = \left\lfloor \frac{f}{2^{F_d - 1}} \right\rfloor \in \{0, 1\}$$

* If $b_{\text{MSB}} = 0$: the element remains in block $B$ at slot $q_{\text{intra}}$.


* If $b_{\text{MSB}} = 1$: the element migrates to buddy block $B \mid (1 \ll d)$ at slot $q_{\text{intra}}$.



### 2.3 Fingerprint Truncation & False Positive Rate

The resident fingerprint is systematically shortened by one bit:


$$f' = f \pmod{2^{F_d - 1}}, \quad F_{d+1} = F_d - 1$$


For a table initialized with base fingerprint width $F_0$ experiencing $d$ doubling events, the theoretical False Positive Rate (FPR) remains bounded by:


$$\text{FPR}(d) \le \alpha \cdot 2^{-(F_0 - d)}$$

---

## 3. Empirical Benchmarks

All benchmarks were collected on an x86_64 host targeting native CPU features (`-C target-cpu=native`).

### 3.1 Intra-Block Microbenchmarks (Criterion)

| Operation | Time | Cycles (@4.0 GHz) | Throughput / Speedup |
| --- | --- | --- | --- |
| **Scalar 48-Byte Scan** | $3.87\text{ ns}$ | $\sim 15.5$ cycles | Baseline |
| **AVX2 48-Byte Scan** | **$0.94\text{ ns}$** | **$\sim 3.8$ cycles** | **$4.10\times$ faster** |
| **Single Block Insert** | $16.51\text{ ns}$ | $\sim 66.0$ cycles | $60.5\text{ Mops/sec}$ |
| **Block Point Query (Hit)** | $4.41\text{ ns}$ | $\sim 17.6$ cycles | $226.7\text{ Mops/sec}$ |
| **Block Point Query (Miss)** | $4.99\text{ ns}$ | $\sim 19.9$ cycles | $200.4\text{ Mops/sec}$ |
| **Filter Point Lookup (Hit)** | $123.3\text{ ns}$ | $\sim 493$ cycles | $8.11\text{ Mops/sec}$ |
| **Filter Point Lookup (Miss)** | $110.2\text{ ns}$ | $\sim 440$ cycles | $9.07\text{ Mops/sec}$ |

### 3.2 Ingestion Tail Latency Under Successive Doublings

Latency distribution measured during continuous ingestion of 25,000 keys across **11 successive global table doublings** (expanding to 1,281 physical blocks):

| Metric | Measured Latency | Baseline Dynamic Filters (CQF / Aleph)

|
| --- | --- | --- |
| **$p50$ (Median)** | **$200\text{ ns}$** ($0.20\ \mu\text{s}$) | $\sim 400\text{ ns}$<br> |
| **$p90$** | **$400\text{ ns}$** ($0.40\ \mu\text{s}$) | $\sim 800\text{ ns}$<br> |
| **$p99$** | **$1,100\text{ ns}$** ($1.10\ \mu\text{s}$) | **$10\text{--}50\text{ ms}$** (Stop-The-World Freeze)

|
| **$p99.9$** | **$5,300\text{ ns}$** ($5.30\ \mu\text{s}$) | **$50\text{--}150\text{ ms}$** (Stop-The-World Freeze)

|
| **Max Peak** | **$147.1\ \mu\text{s}$** | $> 200\text{ ms}$<br> |
| **Ingestion Throughput** | **$2.25\text{ Mops/sec}$** | $< 0.8\text{ Mops/sec}$<br> 

---

## 4. Repository Structure

```text
cc-aleph/
├── .cargo/
│   └── config.toml                  # Compiler target-cpu native vectorization flags
├── Cargo.toml                       # Dependencies and benchmark definitions
├── benches/
│   ├── bench_latency_tail.rs        # p50/p90/p99/p99.9 ingestion latency profiler
│   ├── bench_micro.rs               # Criterion microbenchmarks (AVX2, block queries)
│   └── bench_throughput.rs          # Multithreaded scaling throughput benchmark
├── examples/
│   └── memtable_ingest.rs           # LSM MemTable write-buffer integration simulation
├── src/
│   ├── lib.rs                       # Top-level crate exports and lint flags
│   ├── block/
│   │   ├── layout.rs                # 64-byte cacheline block memory layout
│   │   ├── mod.rs                   # Block abstraction exports
│   │   ├── packing.rs               # Variable-length bitstream manipulation
│   │   └── simd.rs                  # AVX2, SSE, and BMI2 rank-select primitives
│   ├── directory/
│   │   ├── buddy.rs                 # Buddy allocation and bit-sacrifice migration
│   │   ├── mod.rs                   # Two-level extendible directory & worker engine
│   │   └── seqlock.rs               # Optimistic sequence lock (wait-free readers)
│   ├── epoch/
│   │   └── mod.rs                   # Epoch-Based Memory Reclamation wrappers
│   ├── filter.rs                    # Public CcAlephFilter interface
│   └── hash/
│       └── mod.rs                   # 128-bit xxHash3 decomposition and routing
└── tests/
    ├── test_doubling.rs             # 10 successive doublings (1024x scale) FPR verification
    └── test_multithreaded_stress.rs # Concurrent read/write race verification

```

---

## 5. Getting Started

### 5.1 Prerequisites

* Rust toolchain (stable or nightly).


* CPU with AVX2 and BMI2 instruction sets.



Ensure `.cargo/config.toml` includes native microarchitectural flags:

```toml
[build]
rustflags = ["-C", "target-cpu=native"]

```

### 5.2 Building and Running Tests

```bash
# Execute unit and integration tests
cargo test --release

# Run multithreaded stress validation
cargo test --test test_multithreaded_stress -- --nocapture

# Run 10-level directory doubling and FPR bounds verification
cargo test --test test_doubling -- --nocapture

```

### 5.3 Benchmarks and Profiling

```bash
# Run Criterion microbenchmarks (AVX2 vs Scalar, intra-block ops)
cargo bench --bench bench_micro

# Run tail latency ingestion verification (p50, p99, p99.9)
cargo bench --bench bench_latency_tail -- --nocapture

# Run concurrent LSM MemTable write-buffer simulation
cargo run --example memtable_ingest --release

```

---

## 6. Storage Engine Integration Pattern

```rust
use cc_aleph::CcAlephFilter;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize filter with 20-bit base fingerprints (allowing 10+ doublings)
    let filter = CcAlephFilter::with_fingerprint_bits(20);

    // Thread-safe point ingestion (automatically expands without writer stalls)
    filter.insert(b"transaction_key_100293")?;

    // Wait-free point lookup
    if filter.contains(b"transaction_key_100293") {
        // Probable match: proceed to slower storage engine lookup
    } else {
        // 100% Guaranteed true negative; short-circuit disk I/O
    }

    Ok(())
}

```

---

## 7. License

Licensed under either of [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0) or [MIT License](https://opensource.org/licenses/MIT) at your option.

