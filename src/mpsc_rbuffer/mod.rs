//! Multi-producer/single-consumer lock-free ring buffer.
//!
//! Use [`MPSCRBuffer`] to allocate storage, then call `split()` to obtain one
//! [`MPSCConsumer`] and an initial [`MPSCProducer`], which can be cloned for
//! additional producer threads.

pub mod consumer;
pub mod producer;
pub mod rbuffer;

/// MPSC consumer handle.
pub use consumer::MPSCConsumer;
/// MPSC producer handle.
pub use producer::MPSCProducer;
/// MPSC ring buffer storage and split entry point.
pub use rbuffer::MPSCRBuffer;
