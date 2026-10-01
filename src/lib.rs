//! # CC-Aleph: Cache-Partitioned Concurrent Aleph Filter
//!
//! `cc-aleph` is an unbounded, cacheline-aligned Approximate Membership Query (AMQ)
//! filter engineered for zero-stall write ingestion in Log-Structured Merge (LSM)
//! storage engines.
//!
//! ## Architectural Principles
//!
//! * **64-Byte Cacheline Alignment:** Primary storage units (`Block`) align to 64 bytes
//!   (`#[repr(align(64))]`), guaranteeing single-cycle L1-D cache line loads without
//!   cacheline boundary splits.
//! * **Intra-Block SIMD Vectorization:** AVX2 and SSE2 primitives scan 48-byte packed
//!   fingerprint arrays in parallel via `_mm256_cmpeq_epi8`.
//! * **Lock-Free Extendible Directory:** Saturation ($\alpha \ge 0.85$) triggers asynchronous
//!   buddy-block splitting via fingerprint bit-sacrifice, maintaining $O(1)$ lookup complexity.
//! * **Wait-Free Optimistic Readers:** Readers navigate directory pointers and blocks
//!   using sequence locks (`SeqLock`) without blocking concurrent writers or background splits.
//! * **Safe Epoch Reclamation:** Deallocated directory pointers and retired blocks are
//!   reclaimed via Epoch-Based Memory Reclamation (`crossbeam-epoch`).
//!
//! ## Example Usage
//!
//! ```rust
//! use cc_aleph::CcAlephFilter;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Allocate filter with 20-bit initial fingerprints (supports 10+ doublings)
//! let filter = CcAlephFilter::with_fingerprint_bits(20);
//!
//! // Thread-safe ingestion
//! filter.insert(b"transaction_key_100293")?;
//!
//! // Wait-free membership check
//! if filter.contains(b"transaction_key_100293") {
//!     // Probable hit: proceed to storage engine lookup
//! } else {
//!     // Definite false: skip I/O
//! }
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod block;
pub mod directory;
pub mod epoch;
pub mod filter;
pub mod hash;
pub mod c_api;

pub use filter::CcAlephFilter;
