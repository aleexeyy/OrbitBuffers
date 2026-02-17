use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use rbuffer::SPSCRBuffer;
use std::hint::black_box;

const N: usize = 20_000_000;

fn bench_spsc_rbuffer(c: &mut Criterion) {
    c.bench_function("spsc ring buffer throughput", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;

            for _ in 0..iters {
                let mut buffer = SPSCRBuffer::<u64, 1024>::new();
                let (mut producer, mut consumer) =
                    buffer.split().expect("failed to split SPSC buffer");

                let barrier = Arc::new(Barrier::new(3));

                thread::scope(|s| {
                    let b_prod = barrier.clone();
                    let prod_handle = s.spawn(move || {
                        b_prod.wait();
                        for i in 0..N {
                            let value = black_box(i as u64);
                            while producer.try_push(value).is_err() {
                                std::hint::spin_loop();
                            }
                        }
                    });

                    let b_cons = barrier.clone();
                    let cons_handle = s.spawn(move || {
                        b_cons.wait();
                        let mut sum = 0u64;
                        for _ in 0..N {
                            let val;
                            loop {
                                if let Some(v) = consumer.try_pop() {
                                    val = v;
                                    break;
                                }
                                std::hint::spin_loop();
                            }
                            sum = black_box(sum.wrapping_add(val));
                        }
                        black_box(sum);
                    });

                    barrier.wait();
                    let start = Instant::now();

                    prod_handle.join().unwrap();
                    cons_handle.join().unwrap();

                    total += start.elapsed();
                });
            }

            total
        })
    });

    c.bench_function("spsc ring buffer single-thread latency", |b| {
        b.iter(|| {
            let mut buffer = SPSCRBuffer::<u64, 1024>::new();
            let (mut producer, mut consumer) = buffer.split().expect("failed to split SPSC buffer");

            let start = Instant::now();

            for i in 0..N {
                let value = black_box(i as u64);
                while producer.try_push(value).is_err() {
                    std::hint::spin_loop();
                }
                loop {
                    if let Some(v) = consumer.try_pop() {
                        black_box(v);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }

            black_box(start.elapsed());
        })
    });
}

criterion_group!(benches, bench_spsc_rbuffer);
criterion_main!(benches);
