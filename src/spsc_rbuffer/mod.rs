//! Single-producer/single-consumer lock-free ring buffer.
//!
//! Use [`SPSCRBuffer`] to allocate storage, then call `split()` to obtain one
//! [`SPSCProducer`] and one [`SPSCConsumer`].

pub mod consumer;
pub mod producer;
pub mod rbuffer;

/// SPSC consumer handle.
pub use consumer::SPSCConsumer;
/// SPSC producer handle.
pub use producer::SPSCProducer;
/// SPSC ring buffer storage and split entry point.
pub use rbuffer::SPSCRBuffer;
