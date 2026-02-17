use rbuffer::SPSCRBuffer;
use std::hint::{black_box, spin_loop};
use std::sync::Barrier;
use std::thread;

const RING_SIZE: usize = 1024;
const N: usize = 20_000_000;
const WARMUP_ITERS: usize = 5_000_000;

#[cfg(feature = "profiling")]
#[inline(never)]
pub fn hot_loop_marker() {
    core::hint::black_box(());
}

fn main() {
    let mut buffer = SPSCRBuffer::<usize, RING_SIZE>::new();
    let (mut producer, mut consumer) = buffer.split().expect("split failed");
    let barrier = Barrier::new(3);

    thread::scope(|scope| {
        let producer_handle = thread::Builder::new()
            .name("spsc-producer".to_owned())
            .spawn_scoped(scope, || {
                for i in 0..WARMUP_ITERS {
                    #[cfg(feature = "profiling")]
                    hot_loop_marker();
                    let value = black_box(i);
                    while producer.try_push(value).is_err() {
                        spin_loop();
                    }
                }

                barrier.wait();

                for i in 0..N {
                    #[cfg(feature = "profiling")]
                    hot_loop_marker();
                    let value = black_box(i);
                    while producer.try_push(value).is_err() {
                        spin_loop();
                    }
                }
            })
            .expect("failed to spawn producer thread");

        let consumer_handle = thread::Builder::new()
            .name("spsc-consumer".to_owned())
            .spawn_scoped(scope, || {
                for _ in 0..WARMUP_ITERS {
                    #[cfg(feature = "profiling")]
                    hot_loop_marker();
                    let value = loop {
                        if let Some(v) = consumer.try_pop() {
                            break v;
                        }
                        spin_loop();
                    };
                    black_box(value);
                }

                barrier.wait();

                for _ in 0..N {
                    #[cfg(feature = "profiling")]
                    hot_loop_marker();
                    let value = loop {
                        if let Some(v) = consumer.try_pop() {
                            break v;
                        }
                        spin_loop();
                    };
                    black_box(value);
                }
            })
            .expect("failed to spawn consumer thread");

        barrier.wait();

        producer_handle.join().expect("producer thread panicked");
        consumer_handle.join().expect("consumer thread panicked");
    });
}
