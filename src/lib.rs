//! # CC-Aleph
//!
//! A Cache-Partitioned Concurrent Aleph Filter for Zero-Stall Ingestion
//! in Log-Structured Merge (LSM) storage engines.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod block;
pub mod directory;
pub mod epoch;
pub mod filter;
pub mod hash;

pub use filter::CcAlephFilter;
