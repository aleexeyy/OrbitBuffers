# OrbitBuffers

`OrbitBuffers` provides lock-free ring buffers for low-latency, high-throughput communication:

- `SPSCRBuffer<T, S>`: single-producer, single-consumer
- `MPSCRBuffer<T, S>`: multi-producer, single-consumer

Both implementations are fixed-capacity and require power-of-two sizes.

## SPSC Ring Buffer

The SPSC path is optimized for tight 1P1C pipelines with minimal coordination overhead.

Characteristics:

- `no_std` (enabled by default)
- lock-free operation
- stack-allocated storage (fixed-size array)
- spin-based retry on full/empty paths

Basic usage:

1. Create `SPSCRBuffer<T, S>`.
2. Call `split()` to obtain producer and consumer handles.
3. Producer uses `try_push(...)`.
4. Consumer uses `try_pop()`.

## MPSC Ring Buffer

The MPSC path supports many concurrent producers feeding one consumer.

Design overview:

- lock-free producer progress via atomic write reservation
- per-slot sequencing for correctness under producer races
- single consumer advances head/read state

Scaling behavior:

- highest throughput at low producer counts
- throughput decreases as producer count increases due to contention
- larger capacities reduce pressure from frequent wrap/full contention but do not eliminate inter-producer CAS/fetch-add contention

## Performance (Release, Pinned Threads, 64-Byte Payload)

### MPSC Throughput

| Capacity | Producers | Throughput (Melem/s) | Approx. Latency per Operation (ns) |
| --- | --- | ---: | ---: |
| 64 | 1 | 41.89 | 23.87 |
| 64 | 2 | 23.17 | 43.16 |
| 64 | 4 | 11.60 | 86.21 |
| 1024 | 1 | 46.28 | 21.61 |
| 1024 | 2 | 24.08 | 41.53 |
| 1024 | 4 | 14.57 | 68.63 |
| 16384 | 1 | 44.21 | 22.62 |
| 16384 | 2 | 24.22 | 41.29 |
| 16384 | 4 | 15.25 | 65.57 |

MPSC latency benchmark (1 producer, round-trip):

- Total: `~390.50 µs` for `100,000` operations
- Latency: `~3.90 ns/op` (`total_ns / N`)

### SPSC Throughput Summary

| Capacity | Throughput (Melem/s) | Approx. Latency per Operation (ns) |
| --- | ---: | ---: |
| 64 | 91.29 | 10.95 |
| 1024 | 101.12 | 9.89 |
| 16384 | 113.30 | 8.83 |

Average SPSC throughput: `~101.90 Melem/s`  
Average implied time/op from throughput: `~9.81 ns/op`

SPSC latency benchmark (1P1C round-trip):

- Total: `~666.99 µs` for `100,000` operations
- Latency: `~6.67 ns/op` (`total_ns / N`)

## Notes

- Benchmarks were built and run in release mode.
- Threads were pinned to dedicated cores when available.
- Payload size was `64 bytes` (`#[repr(C)] struct Payload([u8; 64]);`).
- Measurements use Criterion sampling/statistics.
- Throughput-implied latency (`1e3 / Melem/s`) and round-trip latency benchmarks are reported separately.

## Build

```bash
cargo build --release
```
