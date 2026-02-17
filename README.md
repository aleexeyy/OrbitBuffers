# OrbitBuffers

`OrbitBuffers` currently provides a single-producer, single-consumer (SPSC) ring buffer for low-latency data transfer between two threads.

## SPSC Ring Buffer

This implementation is:

- `no_std` (enabled by default)
- high-performant
- fully stack-allocated (fixed-size array, no heap allocation)

It is designed for tight producer/consumer loops where predictable throughput and low overhead matter.

## Performance

Current observed performance target:

- Throughput: around **650 million transfers/second**
- Round-trip latency: around **0.8 ns per producer+consumer pair**

These numbers represent steady-state behavior in optimized runs and may vary by CPU, compiler, and runtime conditions.

## Core API

Main types:

- `SPSCRBuffer<T, S>`
- `SingleProducer<'_, T, S>`
- `SingleConsumer<'_, T, S>`

Basic flow:

1. Create the ring buffer with a compile-time size `S`.
2. Call `split()` once to get producer and consumer handles.
3. Producer calls `try_push(...)`.
4. Consumer calls `try_read()`.

## Minimal Example

```rust
use rbuffer::SPSCRBuffer;

let mut buffer = SPSCRBuffer::<u64, 1024>::new();
let (mut producer, mut consumer) = buffer.split().unwrap();

assert!(producer.try_push(42).is_some());
assert_eq!(consumer.try_read(), Some(42));
```

## Constraints

- `S` must be a power of two.
- The buffer is SPSC only: one producer thread and one consumer thread.
- Intended usage is with spin-based retry loops for peak throughput.

## Build

```bash
cargo build --release
```
