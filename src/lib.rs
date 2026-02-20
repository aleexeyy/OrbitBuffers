#![cfg_attr(not(feature = "std"), no_std)]
//! Lock-free fixed-size ring buffers for low-latency message passing.
//!
//! This crate provides:
//! - [`SPSCRBuffer`]: a single-producer/single-consumer ring buffer.
//! - [`MPSCRBuffer`]: a multi-producer/single-consumer ring buffer.
//!
//! Both buffer types require `S` to be a power of two and greater than `1`.
//! Internally, one slot is reserved to disambiguate full and empty states, so
//! effective capacity is `S - 1`.

#[cfg(test)]
extern crate std;

/// Single-producer/single-consumer ring buffer implementation.
pub mod spsc_rbuffer;

/// Multi-producer/single-consumer ring buffer implementation.
pub mod mpsc_rbuffer;

/// Cache-line padding helper used to reduce false sharing.
pub mod cache_padding;

/// Re-export of the SPSC consumer handle.
pub use spsc_rbuffer::{SPSCConsumer, SPSCProducer, SPSCRBuffer};

/// Re-export of MPSC public types.
pub use mpsc_rbuffer::{MPSCConsumer, MPSCProducer, MPSCRBuffer};
