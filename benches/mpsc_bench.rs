use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use core_affinity::CoreId;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rbuffer::mpsc_rbuffer::MPSCRBuffer;
use rbuffer::mpsc_rbuffer::producer::MPSCProducer;

const THROUGHPUT_N_PER_PRODUCER: usize = 1_000_000;
const LATENCY_ITERS_PER_SAMPLE: usize = 100_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Payload([u8; 64]);

#[inline]
fn make_payload(producer_id: usize, sequence: usize) -> Payload {
    let mut bytes = [0u8; 64];
    let tag = ((producer_id as u64) << 32) | (sequence as u64);
    bytes[..8].copy_from_slice(&tag.to_le_bytes());
    Payload(bytes)
}

#[inline]
fn parse_payload(payload: Payload) -> (usize, usize) {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&payload.0[..8]);
    let tag = u64::from_le_bytes(raw);
    ((tag >> 32) as usize, (tag as u32) as usize)
}

#[inline]
fn push_spin<const S: usize>(producer: &mut MPSCProducer<'_, Payload, S>, mut payload: Payload) {
    loop {
        match producer.push(payload) {
            Ok(()) => return,
            Err(value) => {
                payload = value;
                std::hint::spin_loop();
            }
        }
    }
}

#[inline]
fn pin_thread(core_ids: &[CoreId], slot: usize) {
    if let Some(core_id) = core_ids.get(slot).cloned() {
        let _ = core_affinity::set_for_current(core_id);
    }
}

fn logical_core_count() -> usize {
    thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(1)
}

fn producer_counts(_logical_cores: usize) -> Vec<usize> {
    let candidates = [1, 2, 4];
    let mut counts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !counts.contains(&candidate) {
            counts.push(candidate);
        }
    }
    counts
}

fn run_mpsc_timed_case<const S: usize>(
    producer_count: usize,
    n_per_producer: usize,
    core_ids: &[CoreId],
) -> Duration {
    let total_to_consume = producer_count * n_per_producer;
    let mut buffer = MPSCRBuffer::<Payload, S>::new();
    let (first_producer, mut consumer) = buffer.split();

    let mut producers = Vec::with_capacity(producer_count);
    producers.push(first_producer);
    for _ in 1..producer_count {
        let cloned = producers[0].clone();
        producers.push(cloned);
    }

    let barrier = Arc::new(Barrier::new(producer_count + 2));

    thread::scope(|scope| {
        let mut producer_handles = Vec::with_capacity(producer_count);
        for (producer_id, mut producer) in producers.into_iter().enumerate() {
            let start = barrier.clone();
            let core_slot = producer_id;
            producer_handles.push(scope.spawn(move || {
                pin_thread(core_ids, core_slot);
                start.wait();
                for sequence in 0..n_per_producer {
                    let payload = black_box(make_payload(producer_id, sequence));
                    push_spin(&mut producer, payload);
                }
                n_per_producer
            }));
        }

        let start = barrier.clone();
        let consumer_handle = scope.spawn(move || {
            pin_thread(core_ids, producer_count);
            let mut next_expected = vec![0usize; producer_count];
            let mut consumed = 0usize;

            start.wait();
            while consumed < total_to_consume {
                if let Some(payload) = consumer.try_pop() {
                    let (producer_id, sequence) = parse_payload(payload);
                    assert!(
                        producer_id < producer_count,
                        "producer id out of bounds: {producer_id}"
                    );
                    assert_eq!(
                        sequence, next_expected[producer_id],
                        "out-of-order sequence for producer {producer_id}"
                    );
                    next_expected[producer_id] += 1;
                    consumed += 1;
                    continue;
                }
                std::hint::spin_loop();
            }

            for expected in next_expected {
                assert_eq!(expected, n_per_producer, "missing values in consumer");
            }
            assert!(consumer.try_pop().is_none());
            consumed
        });

        barrier.wait();
        let start = Instant::now();

        let mut produced = 0usize;
        for handle in producer_handles {
            produced += handle.join().unwrap();
        }
        let consumed = consumer_handle.join().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(produced, total_to_consume);
        assert_eq!(consumed, total_to_consume);

        let ops_per_sec = (produced as f64) / elapsed.as_secs_f64();
        black_box(ops_per_sec);

        elapsed
    })
}

fn run_throughput_iters<const S: usize>(
    iters: u64,
    producer_count: usize,
    n_per_producer: usize,
    core_ids: &[CoreId],
) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        total += run_mpsc_timed_case::<S>(producer_count, n_per_producer, core_ids);
    }
    total
}

fn bench_throughput_capacity<const S: usize>(
    c: &mut Criterion,
    producer_counts: &[usize],
    core_ids: &[CoreId],
) {
    let mut group = c.benchmark_group(format!("mpsc_throughput_cap_{S}"));
    for &producer_count in producer_counts {
        group.throughput(Throughput::Elements(
            (producer_count * THROUGHPUT_N_PER_PRODUCER) as u64,
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(producer_count),
            &producer_count,
            |b, &producers| {
                b.iter_custom(|iters| {
                    run_throughput_iters::<S>(iters, producers, THROUGHPUT_N_PER_PRODUCER, core_ids)
                });
            },
        );
    }
    group.finish();
}

fn bench_mpsc_throughput(c: &mut Criterion) {
    let cores = logical_core_count();
    let counts = producer_counts(cores);
    let core_ids = core_affinity::get_core_ids().unwrap_or_default();

    bench_throughput_capacity::<64>(c, &counts, &core_ids);
    bench_throughput_capacity::<1024>(c, &counts, &core_ids);
    bench_throughput_capacity::<16384>(c, &counts, &core_ids);
}

fn bench_mpsc_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpsc_latency_1p1c");
    group.throughput(Throughput::Elements(LATENCY_ITERS_PER_SAMPLE as u64));

    group.bench_function("roundtrip_cap_1024", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut buffer = MPSCRBuffer::<Payload, 1024>::new();
                let (mut producer, mut consumer) = buffer.split();
                let sample_start = Instant::now();
                let mut completed_ops = 0usize;

                for sequence in 0..LATENCY_ITERS_PER_SAMPLE {
                    let payload = black_box(make_payload(0, sequence));
                    push_spin(&mut producer, payload);

                    let value = loop {
                        if let Some(v) = consumer.try_pop() {
                            break v;
                        }
                        std::hint::spin_loop();
                    };

                    let (producer_id, seen_sequence) = parse_payload(value);
                    assert_eq!(producer_id, 0);
                    assert_eq!(seen_sequence, sequence);
                    completed_ops += 1;
                }

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

fn criterion_config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(8))
        .sample_size(20)
}

criterion_group!(
    name = benches;
    config = criterion_config();
    targets = bench_mpsc_throughput, bench_mpsc_latency
);
criterion_main!(benches);
