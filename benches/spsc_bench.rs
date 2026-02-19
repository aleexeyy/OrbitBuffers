use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use core_affinity::CoreId;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rbuffer::spsc_rbuffer::SPSCRBuffer;

const THROUGHPUT_N: usize = 5_000_000;
const PIPELINE_N: usize = 50_000_000;
const LATENCY_ITERS_PER_SAMPLE: usize = 100_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Payload([u8; 64]);

#[inline]
fn make_payload(sequence: usize) -> Payload {
    let mut bytes = [0u8; 64];
    let tag = sequence as u64;
    bytes[..8].copy_from_slice(&tag.to_le_bytes());
    Payload(bytes)
}

#[inline]
fn parse_payload(payload: Payload) -> usize {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&payload.0[..8]);
    u64::from_le_bytes(raw) as usize
}

#[inline]
fn pin_current(core: Option<CoreId>) {
    if let Some(core_id) = core {
        let _ = core_affinity::set_for_current(core_id);
    }
}

fn affinity_pair(core_ids: &[CoreId]) -> (Option<CoreId>, Option<CoreId>) {
    let producer_core = core_ids.first().cloned();
    let consumer_core = core_ids.get(1).cloned();

    match (producer_core, consumer_core) {
        (Some(p), Some(c)) if p.id != c.id => (Some(p), Some(c)),
        (Some(p), _) => (Some(p), None),
        _ => (None, None),
    }
}

fn run_spsc_timed_case<const S: usize>(n: usize, core_ids: &[CoreId]) -> Duration {
    let (producer_core, consumer_core) = affinity_pair(core_ids);
    let mut buffer = SPSCRBuffer::<Payload, S>::new();
    let (mut producer, mut consumer) = buffer.split();
    let barrier = Arc::new(Barrier::new(3));

    thread::scope(|scope| {
        let prod_barrier = barrier.clone();
        let producer_handle = scope.spawn(move || {
            pin_current(producer_core);
            prod_barrier.wait();
            for sequence in 0..n {
                let mut payload = black_box(make_payload(sequence));
                loop {
                    match producer.try_push(payload) {
                        Ok(()) => break,
                        Err(value) => {
                            payload = value;
                            std::hint::spin_loop();
                        }
                    }
                }
            }
            n
        });

        let cons_barrier = barrier.clone();
        let consumer_handle = scope.spawn(move || {
            pin_current(consumer_core);
            cons_barrier.wait();
            let mut consumed = 0usize;
            let mut expected_sequence = 0usize;

            while consumed < n {
                if let Some(payload) = consumer.try_pop() {
                    let sequence = parse_payload(black_box(payload));
                    assert_eq!(
                        sequence, expected_sequence,
                        "spsc FIFO violation at index {consumed}"
                    );
                    expected_sequence += 1;
                    consumed += 1;
                    continue;
                }
                std::hint::spin_loop();
            }

            assert_eq!(expected_sequence, n);
            assert!(consumer.try_pop().is_none());
            consumed
        });

        barrier.wait();
        let start = Instant::now();

        let produced = producer_handle.join().unwrap();
        let consumed = consumer_handle.join().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(produced, n);
        assert_eq!(consumed, n);

        let ops_per_sec = (n as f64) / elapsed.as_secs_f64();
        black_box(ops_per_sec);

        elapsed
    })
}

fn run_spsc_iters<const S: usize>(iters: u64, n: usize, core_ids: &[CoreId]) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        total += run_spsc_timed_case::<S>(n, core_ids);
    }
    total
}

fn bench_spsc_throughput(c: &mut Criterion) {
    let core_ids = core_affinity::get_core_ids().unwrap_or_default();
    let mut group = c.benchmark_group("spsc_throughput_1p1c");
    group.throughput(Throughput::Elements(THROUGHPUT_N as u64));

    group.bench_with_input(BenchmarkId::from_parameter(64usize), &64usize, |b, _| {
        b.iter_custom(|iters| run_spsc_iters::<64>(iters, THROUGHPUT_N, &core_ids));
    });
    group.bench_with_input(
        BenchmarkId::from_parameter(1024usize),
        &1024usize,
        |b, _| {
            b.iter_custom(|iters| run_spsc_iters::<1024>(iters, THROUGHPUT_N, &core_ids));
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter(16384usize),
        &16384usize,
        |b, _| {
            b.iter_custom(|iters| run_spsc_iters::<16384>(iters, THROUGHPUT_N, &core_ids));
        },
    );

    group.finish();
}

fn bench_spsc_roundtrip_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc_latency_1p1c_single_thread");
    group.throughput(Throughput::Elements(LATENCY_ITERS_PER_SAMPLE as u64));

    group.bench_function("roundtrip_cap_1024", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;

            for _ in 0..iters {
                let mut buffer = SPSCRBuffer::<Payload, 1024>::new();
                let (mut producer, mut consumer) = buffer.split();
                let mut expected_sequence = 0usize;
                let sample_start = Instant::now();
                let mut completed_ops = 0usize;

                for _ in 0..LATENCY_ITERS_PER_SAMPLE {
                    let mut payload = black_box(make_payload(expected_sequence));

                    loop {
                        match producer.try_push(payload) {
                            Ok(()) => break,
                            Err(value) => {
                                payload = value;
                                std::hint::spin_loop();
                            }
                        }
                    }

                    let popped = loop {
                        if let Some(v) = consumer.try_pop() {
                            break v;
                        }
                        std::hint::spin_loop();
                    };

                    let sequence = parse_payload(black_box(popped));
                    assert_eq!(sequence, expected_sequence);
                    expected_sequence += 1;
                    completed_ops += 1;
                }

                assert_eq!(expected_sequence, LATENCY_ITERS_PER_SAMPLE);
                assert_eq!(completed_ops, LATENCY_ITERS_PER_SAMPLE);
                assert!(consumer.try_pop().is_none());

                let sample_elapsed = sample_start.elapsed();
                let latency_ns_per_op = sample_elapsed.as_nanos() as f64 / (completed_ops as f64);

                black_box(latency_ns_per_op);
                total += sample_elapsed;
            }

            total
        })
    });

    group.finish();
}

fn bench_spsc_batched_pipeline(c: &mut Criterion) {
    let core_ids = core_affinity::get_core_ids().unwrap_or_default();
    let mut group = c.benchmark_group("spsc_batched_pipeline_1p1c");
    group.throughput(Throughput::Elements(PIPELINE_N as u64));

    group.bench_with_input(BenchmarkId::from_parameter(64usize), &64usize, |b, _| {
        b.iter_custom(|iters| run_spsc_iters::<64>(iters, PIPELINE_N, &core_ids));
    });
    group.bench_with_input(
        BenchmarkId::from_parameter(1024usize),
        &1024usize,
        |b, _| {
            b.iter_custom(|iters| run_spsc_iters::<1024>(iters, PIPELINE_N, &core_ids));
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter(16384usize),
        &16384usize,
        |b, _| {
            b.iter_custom(|iters| run_spsc_iters::<16384>(iters, PIPELINE_N, &core_ids));
        },
    );

    group.finish();
}

fn criterion_config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(12))
        .sample_size(20)
}

criterion_group!(
    name = benches;
    config = criterion_config();
    targets = bench_spsc_throughput, bench_spsc_roundtrip_latency, bench_spsc_batched_pipeline
);
criterion_main!(benches);
