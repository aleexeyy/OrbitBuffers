#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
extern crate std;

pub mod spsc_rbuffer;

pub mod mpsc_rbuffer;

pub mod cache_padding;

pub use spsc_rbuffer::{SPSCConsumer, SPSCProducer, SPSCRBuffer};

pub use mpsc_rbuffer::MPSCRBuffer;
