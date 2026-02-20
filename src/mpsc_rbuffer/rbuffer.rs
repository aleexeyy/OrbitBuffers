use crate::cache_padding::CachePadded;
use crate::mpsc_rbuffer::consumer::MPSCConsumer;
use crate::mpsc_rbuffer::producer::MPSCProducer;
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Internal slot used by [`MPSCRBuffer`] for sequence-based coordination.
///
/// This type is public for crate API stability, but is primarily an internal
/// implementation detail and is not intended to be manipulated directly.
pub struct Slot<T>
where
    T: Send,
{
    pub value: MaybeUninit<T>,
    pub sequence: AtomicUsize,
}

impl<T> Default for Slot<T>
where
    T: Send,
{
    fn default() -> Self {
        Self {
            value: MaybeUninit::<T>::uninit(),
            sequence: AtomicUsize::new(0),
        }
    }
}

impl<T> Slot<T>
where
    T: Send,
{
    /// Creates an uninitialized slot with an initial sequence value.
    fn new(i: usize) -> Self {
        Self {
            value: MaybeUninit::<T>::uninit(),
            sequence: AtomicUsize::new(i),
        }
    }
}

/// Storage for a lock-free multi-producer/single-consumer ring buffer.
///
/// Producers reserve write positions atomically and coordinate through per-slot
/// sequence numbers; the single consumer advances read state.
///
/// # Capacity semantics
/// `S` must be a power of two and greater than `1`. Effective user capacity is
/// `S - 1` elements.
pub struct MPSCRBuffer<T, const S: usize>
where
    T: Send,
{
    pub(super) real_write_index: CachePadded<AtomicUsize>,
    // not atomic, cause we only write it from one consumer and dont read it
    pub(super) real_read_index: CachePadded<UnsafeCell<usize>>,
    pub(super) ring: [UnsafeCell<Slot<T>>; S],
}

unsafe impl<T: Send, const S: usize> Sync for MPSCRBuffer<T, S> {}

impl<T, const S: usize> Drop for MPSCRBuffer<T, S>
where
    T: Send,
{
    fn drop(&mut self) {
        let mut read_position = unsafe { *self.real_read_index.0.get() };
        let write_position = self.real_write_index.0.load(Ordering::Relaxed);

        while read_position != write_position {
            let index = read_position & (S - 1);
            let slot = unsafe { &mut *self.ring.get_unchecked(index).get() };

            let sequence = slot.sequence.load(Ordering::Acquire);

            let new_position = read_position.wrapping_add(1);

            if sequence == new_position {
                unsafe { slot.value.assume_init_drop() };
                read_position = new_position;
            } else {
                // should never happen
                panic!("MPSC Buffer sequence is wrong");
            }
        }
    }
}
impl<T, const S: usize> Default for MPSCRBuffer<T, S>
where
    T: Send,
{
    fn default() -> Self {
        assert!(S.is_power_of_two());
        assert!(S > 1);
        Self {
            real_read_index: CachePadded(UnsafeCell::new(0)),
            real_write_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<Slot<T>>, S, _>(|i| {
                UnsafeCell::new(Slot::<T>::new(i))
            }),
        }
    }
}

impl<T, const S: usize> MPSCRBuffer<T, S>
where
    T: Send,
{
    /// Creates an empty MPSC ring buffer.
    ///
    /// Panics if `S` is not a power of two or if `S <= 1`.
    pub fn new() -> Self {
        assert!(S.is_power_of_two());
        assert!(S > 1);
        Self {
            real_read_index: CachePadded(UnsafeCell::new(0)),
            real_write_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<Slot<T>>, S, _>(|i| {
                UnsafeCell::new(Slot::<T>::new(i))
            }),
        }
    }

    /// Splits the buffer into one producer handle and one consumer handle.
    ///
    /// The returned producer can be cloned to create additional producer
    /// handles for other threads.
    pub fn split(&mut self) -> (MPSCProducer<'_, T, S>, MPSCConsumer<'_, T, S>) {
        let read = unsafe { *self.real_read_index.0.get() };

        (
            MPSCProducer { buffer: self },
            MPSCConsumer {
                buffer: self,
                position: read,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpsc_rbuffer::producer::MPSCProducer;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::vec;
    use std::vec::Vec;

    #[inline]
    fn positions<T, const S: usize>(buffer: &MPSCRBuffer<T, S>) -> (usize, usize)
    where
        T: Send,
    {
        let write = buffer.real_write_index.0.load(Ordering::Acquire);
        let read = unsafe { *buffer.real_read_index.0.get() };
        (write, read)
    }

    #[inline]
    fn assert_distance_within_capacity<T, const S: usize>(buffer: &MPSCRBuffer<T, S>)
    where
        T: Send,
    {
        let (write, read) = positions(buffer);
        assert!(
            write >= read,
            "write index regressed: write={write} read={read}"
        );
        assert!(
            write - read <= S,
            "buffer distance exceeded capacity in controlled path: write={write} read={read} size={S}"
        );
    }

    #[derive(Debug)]
    struct DropTracker {
        id: usize,
        per_id: Arc<[AtomicUsize]>,
        total: Arc<AtomicUsize>,
    }

    impl Drop for DropTracker {
        fn drop(&mut self) {
            self.per_id[self.id].fetch_add(1, Ordering::AcqRel);
            self.total.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[derive(Debug)]
    struct CycleDrop<'a> {
        drops: &'a AtomicUsize,
    }

    impl Drop for CycleDrop<'_> {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    #[should_panic]
    fn new_panics_for_non_power_of_two_sizes() {
        let _ = MPSCRBuffer::<u8, 3>::new();
    }

    #[test]
    #[should_panic]
    fn new_panics_for_zero_size() {
        let _ = MPSCRBuffer::<u8, 0>::new();
    }

    #[test]
    #[should_panic]
    fn new_panics_for_one_size() {
        let _ = MPSCRBuffer::<u8, 1>::new();
    }

    #[test]
    fn slot_reuse_across_many_cycles_large_capacity() {
        const SIZE: usize = 4096;
        const CYCLES: usize = 16;

        let mut buffer = MPSCRBuffer::<usize, SIZE>::new();
        let (mut producer, mut consumer) = buffer.split();

        for cycle in 0..CYCLES {
            let base = cycle * SIZE;
            for i in 0..SIZE {
                assert!(producer.push(base + i).is_ok());
            }
            for i in 0..SIZE {
                assert_eq!(consumer.pop(), base + i);
            }
            assert_eq!(consumer.try_pop(), None);
            assert_distance_within_capacity(consumer.buffer);
        }
    }

    #[test]
    fn multi_producer_high_contention_no_loss_no_duplicates() {
        const PRODUCERS: usize = 6;
        const ITEMS_PER_PRODUCER: usize = 20_000;
        const TOTAL: usize = PRODUCERS * ITEMS_PER_PRODUCER;

        let mut buffer = MPSCRBuffer::<u64, 1024>::new();
        let (producer0, mut consumer) = buffer.split();
        let shared = producer0.buffer;

        let barrier = Arc::new(Barrier::new(PRODUCERS + 1));
        let mut next_expected = [0usize; PRODUCERS];
        let mut seen = [0usize; PRODUCERS];

        thread::scope(|scope| {
            let mut first = Some(producer0);
            for pid in 0..PRODUCERS {
                let start = barrier.clone();
                let mut producer = if pid == 0 {
                    first.take().expect("missing first producer")
                } else {
                    MPSCProducer { buffer: shared }
                };

                scope.spawn(move || {
                    start.wait();
                    let mut rnd = 0x9E37_79B9_7F4A_7C15_u64 ^ (pid as u64);
                    for seq in 0..ITEMS_PER_PRODUCER {
                        let value = ((pid as u64) << 32) | (seq as u64);
                        assert!(producer.push(value).is_ok());

                        // Deterministic pseudo-random yielding to perturb interleavings.
                        rnd ^= rnd << 13;
                        rnd ^= rnd >> 7;
                        rnd ^= rnd << 17;
                        if rnd & 0x3F == 0 {
                            thread::yield_now();
                        }
                    }
                });
            }

            barrier.wait();

            for n in 0..TOTAL {
                let value = consumer.pop();
                let pid = (value >> 32) as usize;
                let seq = (value & 0xFFFF_FFFF) as usize;

                assert!(pid < PRODUCERS, "invalid producer id {pid}");
                assert_eq!(
                    seq, next_expected[pid],
                    "out-of-order item for producer {pid}"
                );
                next_expected[pid] += 1;
                seen[pid] += 1;

                if n & 0x1FFF == 0 {
                    thread::yield_now();
                }
            }
        });

        for (pid, s) in seen.iter().enumerate().take(PRODUCERS) {
            assert_eq!(*s, ITEMS_PER_PRODUCER, "missing values from pid={pid}");
        }
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn multi_producer_stress_repeated_runs() {
        const RUNS: usize = 24;
        const PRODUCERS: usize = 4;
        const ITEMS_PER_PRODUCER: usize = 3_000;
        const TOTAL: usize = PRODUCERS * ITEMS_PER_PRODUCER;

        for run in 0..RUNS {
            let mut buffer = MPSCRBuffer::<u64, 256>::new();
            let (producer0, mut consumer) = buffer.split();
            let shared = producer0.buffer;
            let barrier = Arc::new(Barrier::new(PRODUCERS + 1));
            let mut seen = vec![0usize; TOTAL];

            thread::scope(|scope| {
                let mut first = Some(producer0);
                for pid in 0..PRODUCERS {
                    let start = barrier.clone();
                    let mut producer = if pid == 0 {
                        first.take().expect("missing first producer")
                    } else {
                        MPSCProducer { buffer: shared }
                    };

                    scope.spawn(move || {
                        start.wait();
                        for seq in 0..ITEMS_PER_PRODUCER {
                            let id = pid * ITEMS_PER_PRODUCER + seq;
                            assert!(producer.push(id as u64).is_ok());
                            if ((seq + pid + run) & 0x7F) == 0 {
                                thread::yield_now();
                            }
                        }
                    });
                }

                barrier.wait();
                for _ in 0..TOTAL {
                    let id = consumer.pop() as usize;
                    assert!(id < TOTAL, "out-of-range id {id}");
                    seen[id] += 1;
                }
            });

            for (id, count) in seen.into_iter().enumerate() {
                assert_eq!(count, 1, "duplicate/lost element at id={id}, run={run}");
            }
            assert_eq!(consumer.try_pop(), None);
        }
    }

    #[test]
    fn drop_with_remaining_elements_drops_exactly_once() {
        const N: usize = 96;
        let per_id: Arc<[AtomicUsize]> = (0..N)
            .map(|_| AtomicUsize::new(0))
            .collect::<Vec<_>>()
            .into();
        let total = Arc::new(AtomicUsize::new(0));

        {
            let mut buffer = MPSCRBuffer::<DropTracker, 128>::new();
            let (mut producer, mut consumer) = buffer.split();

            for id in 0..N {
                let value = DropTracker {
                    id,
                    per_id: per_id.clone(),
                    total: total.clone(),
                };
                assert!(producer.push(value).is_ok());
            }

            let mut consumed = Vec::with_capacity(32);
            for _ in 0..32 {
                consumed.push(consumer.pop());
            }
            drop(consumed);
        }

        assert_eq!(total.load(Ordering::Acquire), N);
        for id in 0..N {
            let drops = per_id[id].load(Ordering::Acquire);
            assert_eq!(drops, 1, "value id={id} dropped {drops} times");
        }
    }

    #[test]
    fn repeated_create_destroy_cycles_drop_all_values_once() {
        const CYCLES: usize = 200;
        let drops = AtomicUsize::new(0);
        let mut created = 0usize;

        for cycle in 0..CYCLES {
            let mut buffer = MPSCRBuffer::<CycleDrop<'_>, 32>::new();
            let (mut producer, mut consumer) = buffer.split();

            let to_push = (cycle % 24) + 1;
            let to_pop = to_push / 3;

            for _ in 0..to_push {
                created += 1;
                assert!(producer.push(CycleDrop { drops: &drops }).is_ok());
            }

            for _ in 0..to_pop {
                let value = consumer.pop();
                drop(value);
            }
        }

        assert_eq!(drops.load(Ordering::Acquire), created);
    }
}
