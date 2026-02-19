use super::consumer::SPSCConsumer;
use super::producer::SPSCProducer;
use crate::cache_padding::CachePadded;
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

pub struct SPSCRBuffer<T, const S: usize>
where
    T: Send,
{
    pub(super) real_write_index: CachePadded<AtomicUsize>,
    pub(super) real_read_index: CachePadded<AtomicUsize>,
    pub(super) ring: [UnsafeCell<MaybeUninit<T>>; S],
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
        assert!(S > 1);
        Self {
            real_write_index: CachePadded(AtomicUsize::new(0)),
            real_read_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<MaybeUninit<T>>, S, _>(|_| {
                UnsafeCell::new(MaybeUninit::<T>::uninit())
            }),
        }
    }
}

impl<T, const S: usize> SPSCRBuffer<T, S>
where
    T: Send,
{
    pub fn new() -> Self {
        assert!(S.is_power_of_two());
        assert!(S > 1);
        Self {
            real_write_index: CachePadded(AtomicUsize::new(0)),
            real_read_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<MaybeUninit<T>>, S, _>(|_| {
                UnsafeCell::new(MaybeUninit::<T>::uninit())
            }),
        }
    }

    pub fn split(&mut self) -> (SPSCProducer<'_, T, S>, SPSCConsumer<'_, T, S>) {
        let write = self.real_write_index.0.load(Ordering::Relaxed);
        let read = self.real_read_index.0.load(Ordering::Relaxed);
        (
            SPSCProducer {
                buffer: self,
                write_index: write,
                cached_read_index: read,
            },
            SPSCConsumer {
                buffer: self,
                cached_write_index: write,
                read_index: read,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
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
        producer: &SPSCProducer<'_, T, S>,
        expected_len: usize,
    ) where
        T: Send,
    {
        assert_state_buffer(producer.buffer, expected_len);
    }

    fn assert_state_from_consumer<T, const S: usize>(
        consumer: &SPSCConsumer<'_, T, S>,
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
    fn new_panics_for_one_size() {
        let _ = SPSCRBuffer::<u8, 1>::new();
    }

    #[test]
    fn unread_values_are_dropped_exactly_once_on_buffer_drop() {
        let drops = AtomicUsize::new(0);
        let clones = AtomicUsize::new(0);

        {
            let mut buffer = SPSCRBuffer::<DropTracker<'_>, 8>::new();
            let (mut producer, _consumer) = buffer.split();

            for id in 0..5 {
                assert!(
                    producer
                        .try_push(DropTracker::new(id, &drops, &clones))
                        .is_ok()
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
            let (mut producer, mut consumer) = buffer.split();

            for id in 0..4 {
                assert!(
                    producer
                        .try_push(DropTracker::new(id, &drops, &clones))
                        .is_ok()
                );
            }
            assert_state_from_producer(&producer, 4);

            let v0 = consumer.try_pop().expect("value should be present");
            let v1 = consumer.try_pop().expect("value should be present");
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
            let (mut producer, _consumer) = buffer.split();
            for id in 0..3 {
                assert!(
                    producer
                        .try_push(DropTracker::new(id, &drops, &clones))
                        .is_ok()
                );
            }
            panic!("forced unwind");
        }));

        assert!(result.is_err(), "panic should be captured");
        assert_eq!(drops.load(Ordering::Acquire), 3);
        assert_eq!(clones.load(Ordering::Acquire), 0);
    }

    #[test]
    fn split_starts_from_current_live_indices_not_zero() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        {
            let (mut producer, mut consumer) = buffer.split();
            assert_eq!(producer.try_push(1), Ok(()));
            assert_eq!(producer.try_push(2), Ok(()));
            assert_eq!(producer.try_push(3), Ok(()));
            assert_eq!(consumer.try_pop(), Some(1));
            assert_eq!(consumer.try_pop(), Some(2));
            assert_eq!(consumer.try_pop(), Some(3));
            assert_state_from_consumer(&consumer, 0);
        }

        let expected_write = buffer.real_write_index.0.load(Ordering::Acquire);
        let expected_read = buffer.real_read_index.0.load(Ordering::Acquire);
        assert_eq!(expected_write, expected_read);
        assert!(expected_write != 0, "indices should have advanced");

        let (producer, consumer) = buffer.split();
        assert_eq!(producer.write_index, expected_write);
        assert_eq!(producer.cached_read_index, expected_read);
        assert_eq!(consumer.read_index, expected_read);
        assert_eq!(consumer.cached_write_index, expected_write);
    }

    #[test]
    fn sequential_resplit_stress_preserves_split_invariant() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();

        for cycle in 0..128_u32 {
            let (mut producer, mut consumer) = buffer.split();
            assert_eq!(producer.try_push(cycle), Ok(()));
            assert_eq!(consumer.try_pop(), Some(cycle));
            assert_state_from_consumer(&consumer, 0);
            drop(producer);
            drop(consumer);

            let expected_write = buffer.real_write_index.0.load(Ordering::Acquire);
            let expected_read = buffer.real_read_index.0.load(Ordering::Acquire);
            assert_eq!(expected_write, expected_read);

            let (producer2, consumer2) = buffer.split();
            assert_eq!(producer2.write_index, expected_write, "cycle={cycle}");
            assert_eq!(producer2.cached_read_index, expected_read, "cycle={cycle}");
            assert_eq!(consumer2.read_index, expected_read, "cycle={cycle}");
            assert_eq!(
                consumer2.cached_write_index, expected_write,
                "cycle={cycle}"
            );
        }
    }
}
