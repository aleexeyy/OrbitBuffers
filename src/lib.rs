#![cfg_attr(not(feature = "std"), no_std)]
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
mod cache_padding;
use cache_padding::CachePadded;
#[cfg(test)]
extern crate std;

pub struct SingleProducer<'a, T, const S: usize>
where
    T: Send,
{
    buffer: &'a SPSCRBuffer<T, S>,
    write_index: usize,
    cached_read_index: usize,
}

impl<'a, T, const S: usize> Drop for SingleProducer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> SingleProducer<'a, T, S>
where
    T: Send,
{
    #[cfg_attr(feature = "profiling", inline(never))]
    #[cfg_attr(not(feature = "profiling"), inline(always))]
    pub fn try_push(&mut self, data: T) -> Option<()> {
        let next_write_index = (self.write_index + 1) & (S - 1);

        //check wether buffer is full and update cache if necessary
        if next_write_index == self.cached_read_index {
            self.cached_read_index = self.buffer.real_read_index.0.load(Ordering::Acquire);

            if next_write_index == self.cached_read_index {
                return None;
            }
        }

        // safety: the old value is dropped by consumer
        let _ = unsafe { (*self.buffer.ring[self.write_index].get()).write(data) };

        self.buffer
            .real_write_index
            .0
            .store(next_write_index, Ordering::Release);

        self.write_index = next_write_index;

        Some(())
    }
}

pub struct SingleConsumer<'a, T, const S: usize>
where
    T: Send,
{
    buffer: &'a SPSCRBuffer<T, S>,
    cached_write_index: usize,
    read_index: usize,
}

impl<'a, T, const S: usize> Drop for SingleConsumer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> SingleConsumer<'a, T, S>
where
    T: Send,
{
    #[cfg_attr(feature = "profiling", inline(never))]
    #[cfg_attr(not(feature = "profiling"), inline(always))]
    pub fn try_read(&mut self) -> Option<T> {
        if self.read_index == self.cached_write_index {
            self.cached_write_index = self.buffer.real_write_index.0.load(Ordering::Acquire);

            if self.read_index == self.cached_write_index {
                return None;
            }
        }
        let value = unsafe { (*self.buffer.ring[self.read_index].get()).assume_init_read() };

        self.read_index = (self.read_index + 1) & (S - 1);

        self.buffer
            .real_read_index
            .0
            .store(self.read_index, Ordering::Release);

        Some(value)
    }
}

pub struct SPSCRBuffer<T, const S: usize>
where
    T: Send,
{
    real_write_index: CachePadded<AtomicUsize>,
    real_read_index: CachePadded<AtomicUsize>,
    ring: [UnsafeCell<MaybeUninit<T>>; S],
}

unsafe impl<T: Send, const S: usize> Sync for SPSCRBuffer<T, S> {}

impl<T, const S: usize> Drop for SPSCRBuffer<T, S>
where
    T: Send,
{
    fn drop(&mut self) {
        let mut read = self.real_read_index.0.load(Ordering::Relaxed);

        let write = self.real_write_index.0.load(Ordering::Relaxed);

        while read != write {
            unsafe {
                (*self.ring[read].get()).assume_init_drop();
            }

            read = (read + 1) & (S - 1);
        }
    }
}
impl<T, const S: usize> Default for SPSCRBuffer<T, S>
where
    T: Send,
{
    fn default() -> Self {
        assert!(S.is_power_of_two());
        Self {
            ring: core::array::from_fn::<UnsafeCell<MaybeUninit<T>>, S, _>(|_| {
                UnsafeCell::new(MaybeUninit::<T>::uninit())
            }),
            real_write_index: CachePadded(AtomicUsize::new(0)),
            real_read_index: CachePadded(AtomicUsize::new(0)),
        }
    }
}

impl<T, const S: usize> SPSCRBuffer<T, S>
where
    T: Send,
{
    pub fn new() -> Self {
        assert!(S.is_power_of_two());
        Self {
            ring: core::array::from_fn::<UnsafeCell<MaybeUninit<T>>, S, _>(|_| {
                UnsafeCell::new(MaybeUninit::<T>::uninit())
            }),
            real_write_index: CachePadded(AtomicUsize::new(0)),
            real_read_index: CachePadded(AtomicUsize::new(0)),
        }
    }

    pub fn split(&mut self) -> Option<(SingleProducer<'_, T, S>, SingleConsumer<'_, T, S>)> {
        let write = self.real_write_index.0.load(Ordering::Relaxed);
        let read = self.real_read_index.0.load(Ordering::Relaxed);
        Some((
            SingleProducer {
                buffer: self,
                write_index: write,
                cached_read_index: read,
            },
            SingleConsumer {
                buffer: self,
                cached_write_index: write,
                read_index: read,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[derive(Debug)]
    struct DropTracker<'a> {
        id: u32,
        drops: &'a AtomicUsize,
        clones: &'a AtomicUsize,
    }

    impl<'a> DropTracker<'a> {
        fn new(id: u32, drops: &'a AtomicUsize, clones: &'a AtomicUsize) -> Self {
            Self { id, drops, clones }
        }
    }

    impl Clone for DropTracker<'_> {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::Relaxed);
            Self {
                id: self.id,
                drops: self.drops,
                clones: self.clones,
            }
        }
    }

    impl Drop for DropTracker<'_> {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn occupied_len<T, const S: usize>(buffer: &SPSCRBuffer<T, S>) -> usize
    where
        T: Send,
    {
        let write = buffer.real_write_index.0.load(Ordering::Acquire);
        let read = buffer.real_read_index.0.load(Ordering::Acquire);
        write.wrapping_sub(read) & S.wrapping_sub(1)
    }

    fn assert_state_buffer<T, const S: usize>(buffer: &SPSCRBuffer<T, S>, expected_len: usize)
    where
        T: Send,
    {
        let write = buffer.real_write_index.0.load(Ordering::Acquire);
        let read = buffer.real_read_index.0.load(Ordering::Acquire);
        assert!(write < S, "write index out of bounds: {write} >= {S}");
        assert!(read < S, "read index out of bounds: {read} >= {S}");

        let len = occupied_len(buffer);
        assert_eq!(len, expected_len, "unexpected occupancy");
        assert!(
            len <= S.saturating_sub(1),
            "occupancy exceeds ring capacity: {len} > {}",
            S.saturating_sub(1)
        );
    }

    fn assert_state_from_producer<T, const S: usize>(
        producer: &SingleProducer<'_, T, S>,
        expected_len: usize,
    ) where
        T: Send,
    {
        assert_state_buffer(producer.buffer, expected_len);
    }

    fn assert_state_from_consumer<T, const S: usize>(
        consumer: &SingleConsumer<'_, T, S>,
        expected_len: usize,
    ) where
        T: Send,
    {
        assert_state_buffer(consumer.buffer, expected_len);
    }

    #[test]
    #[should_panic]
    fn new_panics_for_non_power_of_two_sizes() {
        let _ = SPSCRBuffer::<u8, 3>::new();
    }

    #[test]
    #[should_panic]
    fn new_panics_for_zero_size() {
        let _ = SPSCRBuffer::<u8, 0>::new();
    }

    #[test]
    fn size_one_buffer_has_zero_usable_capacity() {
        let mut buffer = SPSCRBuffer::<u8, 1>::new();
        let (mut producer, mut consumer) = buffer.split().expect("split should succeed");

        assert_eq!(producer.try_push(7), None);
        assert_eq!(consumer.try_read(), None);
        assert_state_from_producer(&producer, 0);
    }

    #[test]
    fn read_from_empty_is_idempotent_and_keeps_internal_state() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split().expect("split should succeed");

        for _ in 0..64 {
            assert_eq!(consumer.try_read(), None);
            assert_state_from_consumer(&consumer, 0);
        }

        assert_eq!(producer.try_push(42), Some(()));
        assert_state_from_producer(&producer, 1);
        assert_eq!(consumer.try_read(), Some(42));
        assert_state_from_consumer(&consumer, 0);
        assert_eq!(consumer.try_read(), None);
        assert_state_from_consumer(&consumer, 0);
    }

    #[test]
    fn full_buffer_rejects_push_without_mutating_indices_and_drops_argument() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
        let (mut producer, mut consumer) = buffer.split().expect("split should succeed");

        for id in 0..7 {
            assert_eq!(
                producer.try_push(DropTracker::new(id, &drops, &clones)),
                Some(())
            );
        }
        assert_state_from_producer(&producer, 7);

        let before_write = producer.buffer.real_write_index.0.load(Ordering::Acquire);
        let before_read = producer.buffer.real_read_index.0.load(Ordering::Acquire);
        assert_eq!(
            producer.try_push(DropTracker::new(999, &drops, &clones)),
            None
        );
        assert_eq!(drops.load(Ordering::Acquire), 1);
        assert_eq!(clones.load(Ordering::Acquire), 0);
        assert_eq!(
            producer.buffer.real_write_index.0.load(Ordering::Acquire),
            before_write
        );
        assert_eq!(
            producer.buffer.real_read_index.0.load(Ordering::Acquire),
            before_read
        );
        assert_state_from_producer(&producer, 7);

        for expected in 0..7 {
            let value = consumer.try_read().expect("value should be present");
            assert_eq!(value.id, expected);
            drop(value);
        }
        assert!(consumer.try_read().is_none());
        assert_state_from_consumer(&consumer, 0);
        assert_eq!(clones.load(Ordering::Acquire), 0);
        assert_eq!(drops.load(Ordering::Acquire), 8);
    }

    #[test]
    fn unread_values_are_dropped_exactly_once_on_buffer_drop() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        {
            let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
            let (mut producer, _consumer) = buffer.split().expect("split should succeed");

            for id in 0..5 {
                assert_eq!(
                    producer.try_push(DropTracker::new(id, &drops, &clones)),
                    Some(())
                );
            }

            assert_eq!(drops.load(Ordering::Acquire), 0);
            assert_eq!(clones.load(Ordering::Acquire), 0);
        }

        assert_eq!(drops.load(Ordering::Acquire), 5);
        assert_eq!(clones.load(Ordering::Acquire), 0);
    }

    #[test]
    fn partially_read_values_are_not_double_dropped() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        {
            let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
            let (mut producer, mut consumer) = buffer.split().expect("split should succeed");

            for id in 0..4 {
                assert_eq!(
                    producer.try_push(DropTracker::new(id, &drops, &clones)),
                    Some(())
                );
            }
            assert_state_from_producer(&producer, 4);

            let v0 = consumer.try_read().expect("value should be present");
            let v1 = consumer.try_read().expect("value should be present");
            assert_eq!(v0.id, 0);
            assert_eq!(v1.id, 1);
            drop(v0);
            drop(v1);
            assert_eq!(drops.load(Ordering::Acquire), 2);
            assert_state_from_consumer(&consumer, 2);
        }

        assert_eq!(drops.load(Ordering::Acquire), 4);
        assert_eq!(clones.load(Ordering::Acquire), 0);
    }

    #[test]
    fn panic_unwind_still_cleans_buffered_values() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
            let (mut producer, _consumer) = buffer.split().expect("split should succeed");
            for id in 0..3 {
                assert_eq!(
                    producer.try_push(DropTracker::new(id, &drops, &clones)),
                    Some(())
                );
            }
            panic!("forced unwind");
        }));

        assert!(result.is_err(), "panic should be captured");
        assert_eq!(drops.load(Ordering::Acquire), 3);
        assert_eq!(clones.load(Ordering::Acquire), 0);
    }

    #[test]
    fn dropping_producer_does_not_invalidate_consumer() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split().expect("split should succeed");

        for value in [11_u32, 22, 33] {
            assert_eq!(producer.try_push(value), Some(()));
        }
        assert_state_from_producer(&producer, 3);

        drop(producer);
        assert_eq!(consumer.try_read(), Some(11));
        assert_state_from_consumer(&consumer, 2);
        assert_eq!(consumer.try_read(), Some(22));
        assert_state_from_consumer(&consumer, 1);
        assert_eq!(consumer.try_read(), Some(33));
        assert_state_from_consumer(&consumer, 0);
        assert_eq!(consumer.try_read(), None);
        assert_state_from_consumer(&consumer, 0);
    }

    #[test]
    fn dropping_consumer_does_not_invalidate_producer_or_cleanup() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        {
            let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
            let (mut producer, consumer) = buffer.split().expect("split should succeed");
            drop(consumer);

            for id in 0..7 {
                assert_eq!(
                    producer.try_push(DropTracker::new(id, &drops, &clones)),
                    Some(())
                );
            }
            assert_state_from_producer(&producer, 7);

            assert_eq!(
                producer.try_push(DropTracker::new(777, &drops, &clones)),
                None
            );
            assert_eq!(drops.load(Ordering::Acquire), 1);
            assert_eq!(clones.load(Ordering::Acquire), 0);
            assert_state_from_producer(&producer, 7);
        }

        assert_eq!(drops.load(Ordering::Acquire), 8);
        assert_eq!(clones.load(Ordering::Acquire), 0);
    }

    #[test]
    fn split_starts_from_current_live_indices_not_zero() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        {
            let (mut producer, mut consumer) = buffer.split().expect("split should succeed");
            assert_eq!(producer.try_push(1), Some(()));
            assert_eq!(producer.try_push(2), Some(()));
            assert_eq!(producer.try_push(3), Some(()));
            assert_eq!(consumer.try_read(), Some(1));
            assert_eq!(consumer.try_read(), Some(2));
            assert_eq!(consumer.try_read(), Some(3));
            assert_state_from_consumer(&consumer, 0);
        }

        let expected_write = buffer.real_write_index.0.load(Ordering::Acquire);
        let expected_read = buffer.real_read_index.0.load(Ordering::Acquire);
        assert_eq!(expected_write, expected_read);
        assert!(expected_write != 0, "indices should have advanced");

        let (producer, consumer) = buffer.split().expect("split should succeed");
        assert_eq!(producer.write_index, expected_write);
        assert_eq!(producer.cached_read_index, expected_read);
        assert_eq!(consumer.read_index, expected_read);
        assert_eq!(consumer.cached_write_index, expected_write);
    }

    #[test]
    fn sequential_resplit_stress_preserves_split_invariant() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();

        for cycle in 0..128_u32 {
            let (mut producer, mut consumer) = buffer.split().expect("split should succeed");
            assert_eq!(producer.try_push(cycle), Some(()));
            assert_eq!(consumer.try_read(), Some(cycle));
            assert_state_from_consumer(&consumer, 0);
            drop(producer);
            drop(consumer);

            let expected_write = buffer.real_write_index.0.load(Ordering::Acquire);
            let expected_read = buffer.real_read_index.0.load(Ordering::Acquire);
            assert_eq!(expected_write, expected_read);

            let (producer2, consumer2) = buffer.split().expect("split should succeed");
            assert_eq!(producer2.write_index, expected_write, "cycle={cycle}");
            assert_eq!(producer2.cached_read_index, expected_read, "cycle={cycle}");
            assert_eq!(consumer2.read_index, expected_read, "cycle={cycle}");
            assert_eq!(
                consumer2.cached_write_index, expected_write,
                "cycle={cycle}"
            );
        }
    }

    #[test]
    fn deterministic_model_stress_preserves_fifo_and_occupancy() {
        const SIZE: usize = 8;
        const CAPACITY: usize = SIZE - 1;
        const STEPS: usize = 20_000;

        let mut buffer = SPSCRBuffer::<u32, SIZE>::new();
        let (mut producer, mut consumer) = buffer.split().expect("split should succeed");
        let mut model = VecDeque::<u32>::with_capacity(CAPACITY);

        let mut seed = 0xF00D_F00D_CAFE_BABE_u64;
        for step in 0..STEPS {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let op = (seed >> 33) % 4;

            match op {
                0 | 1 => {
                    let value = step as u32;
                    let expected = if model.len() < CAPACITY {
                        model.push_back(value);
                        Some(())
                    } else {
                        None
                    };
                    assert_eq!(producer.try_push(value), expected);
                }
                _ => {
                    let expected = model.pop_front();
                    assert_eq!(consumer.try_read(), expected);
                }
            }

            assert_state_from_producer(&producer, model.len());
        }

        while let Some(expected) = model.pop_front() {
            assert_eq!(consumer.try_read(), Some(expected));
            assert_state_from_consumer(&consumer, model.len());
        }
        assert_eq!(consumer.try_read(), None);
        assert_state_from_consumer(&consumer, 0);
    }
}
