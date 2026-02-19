use super::rbuffer::SPSCRBuffer;
use core::hint::spin_loop;
use core::sync::atomic::Ordering;

pub struct SPSCProducer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a SPSCRBuffer<T, S>,
    pub(super) write_index: usize,
    pub(super) cached_read_index: usize,
}

// implement drop to silence clippy warnings
impl<'a, T, const S: usize> Drop for SPSCProducer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> SPSCProducer<'a, T, S>
where
    T: Send,
{
    #[inline]
    pub const fn capacity(&self) -> usize {
        S - 1
    }

    #[inline]
    pub fn free_space(&mut self) -> usize {
        let free = (self
            .cached_read_index
            .wrapping_sub(self.write_index)
            .wrapping_sub(1))
            & (S - 1);

        if free != 0 {
            return free;
        }

        self.cached_read_index = self.buffer.real_read_index.0.load(Ordering::Acquire);

        (self
            .cached_read_index
            .wrapping_sub(self.write_index)
            .wrapping_sub(1))
            & (S - 1)
    }

    #[inline]
    pub fn is_full(&mut self) -> bool {
        self.free_space() == 0
    }

    #[inline]
    pub fn push(&mut self, mut data: T) {
        loop {
            match self.try_push(data) {
                Ok(()) => return,
                Err(value) => {
                    data = value;
                    spin_loop();
                }
            }
        }
    }

    #[cfg_attr(feature = "profiling", inline(never))]
    #[cfg_attr(not(feature = "profiling"), inline(always))]
    pub fn try_push(&mut self, data: T) -> Result<(), T> {
        let next_write_index = (self.write_index + 1) & (S - 1);

        //check wether buffer is full and update cache if necessary
        if next_write_index == self.cached_read_index {
            self.cached_read_index = self.buffer.real_read_index.0.load(Ordering::Acquire);

            if next_write_index == self.cached_read_index {
                return Err(data);
            }
        }

        // safety: the old value is dropped by consumer
        unsafe { (&mut *self.buffer.ring.get_unchecked(self.write_index).get()).write(data) };

        self.write_index = next_write_index;

        self.buffer
            .real_write_index
            .0
            .store(next_write_index, Ordering::Release);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn occupied_len<T, const S: usize>(buffer: &SPSCRBuffer<T, S>) -> usize
    where
        T: Send,
    {
        let write = buffer.real_write_index.0.load(Ordering::Acquire);
        let read = buffer.real_read_index.0.load(Ordering::Acquire);
        write.wrapping_sub(read) & S.wrapping_sub(1)
    }

    fn assert_state_from_producer<T, const S: usize>(
        producer: &SPSCProducer<'_, T, S>,
        expected_len: usize,
    ) where
        T: Send,
    {
        let write = producer.buffer.real_write_index.0.load(Ordering::Acquire);
        let read = producer.buffer.real_read_index.0.load(Ordering::Acquire);
        assert!(write < S, "write index out of bounds: {write} >= {S}");
        assert!(read < S, "read index out of bounds: {read} >= {S}");

        let len = occupied_len(producer.buffer);
        assert_eq!(len, expected_len, "unexpected occupancy");
        assert!(
            len <= S.saturating_sub(1),
            "occupancy exceeds ring capacity: {len} > {}",
            S.saturating_sub(1)
        );
    }

    #[test]
    fn producer_capacity_and_free_space_without_consumer_progress() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        assert_eq!(producer.capacity(), 7);
        assert_eq!(producer.free_space(), 7);
        assert!(!producer.is_full());

        for i in 0..7 {
            assert_eq!(producer.try_push(i), Ok(()));
            assert_eq!(producer.free_space(), 6 - i as usize);
        }

        assert!(producer.is_full());
        assert_eq!(producer.free_space(), 0);
        assert_state_from_producer(&producer, 7);

        for expected in 0..7 {
            assert_eq!(consumer.pop(), expected);
        }
        assert_state_from_producer(&producer, 0);
    }

    #[test]
    fn try_push_when_full_returns_original_value_and_preserves_indices() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for i in 0..7 {
            assert_eq!(producer.try_push(i), Ok(()));
        }

        let write_before = producer.write_index;
        let cached_read_before = producer.cached_read_index;
        let real_write_before = producer.buffer.real_write_index.0.load(Ordering::Acquire);
        let real_read_before = producer.buffer.real_read_index.0.load(Ordering::Acquire);

        assert_eq!(producer.try_push(777), Err(777));

        assert_eq!(producer.write_index, write_before);
        assert_eq!(
            producer.buffer.real_write_index.0.load(Ordering::Acquire),
            real_write_before
        );
        assert_eq!(
            producer.buffer.real_read_index.0.load(Ordering::Acquire),
            real_read_before
        );

        // Cache may refresh during full check, but must stay in-range.
        assert!(producer.cached_read_index < 8);
        assert!(
            producer.cached_read_index == cached_read_before
                || producer.cached_read_index == real_read_before
        );

        for expected in 0..7 {
            assert_eq!(consumer.pop(), expected);
        }
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn free_space_refreshes_after_consumer_makes_room_from_full() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for i in 0..7 {
            assert_eq!(producer.try_push(i), Ok(()));
        }
        assert!(producer.is_full());
        assert_eq!(producer.free_space(), 0);

        assert_eq!(consumer.pop(), 0);
        assert_eq!(consumer.pop(), 1);
        assert_eq!(consumer.pop(), 2);

        assert_eq!(producer.free_space(), 3);
        assert!(!producer.is_full());

        assert_eq!(producer.try_push(100), Ok(()));
        assert_eq!(producer.try_push(101), Ok(()));
        assert_eq!(producer.try_push(102), Ok(()));
        assert!(producer.is_full());

        for expected in [3_u32, 4, 5, 6, 100, 101, 102] {
            assert_eq!(consumer.pop(), expected);
        }
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn push_blocks_until_consumer_makes_space() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for i in 0..7 {
            assert_eq!(producer.try_push(i), Ok(()));
        }
        assert!(producer.is_full());

        let barrier = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let start = barrier.clone();
            let push_handle = scope.spawn(move || {
                start.wait();
                producer.push(999);
                producer
            });

            barrier.wait();
            assert_eq!(consumer.pop(), 0);

            let _producer = push_handle.join().expect("producer thread panicked");
        });

        for expected in [1_u32, 2, 3, 4, 5, 6, 999] {
            assert_eq!(consumer.pop(), expected);
        }
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn producer_wraparound_preserves_fifo_across_many_cycles() {
        let mut buffer = SPSCRBuffer::<u64, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for cycle in 0..256_u64 {
            let base = cycle * 7;
            for i in 0..7 {
                assert_eq!(producer.try_push(base + i), Ok(()));
            }
            assert!(producer.is_full());

            for i in 0..7 {
                assert_eq!(consumer.pop(), base + i);
            }
            assert_eq!(consumer.try_pop(), None);
            assert_eq!(producer.free_space(), 7);
        }
    }
}
