use super::rbuffer::SPSCRBuffer;
use core::hint::spin_loop;
use core::sync::atomic::Ordering;

pub struct SPSCConsumer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a SPSCRBuffer<T, S>,
    pub(super) cached_write_index: usize,
    pub(super) read_index: usize,
}

// implement drop to silence clippy warnings
impl<'a, T, const S: usize> Drop for SPSCConsumer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> SPSCConsumer<'a, T, S>
where
    T: Send,
{
    #[inline]
    pub const fn capacity(&self) -> usize {
        S - 1
    }

    #[inline]
    pub fn len(&mut self) -> usize {
        let len = self.cached_write_index.wrapping_sub(self.read_index) & (S - 1);
        if len != 0 {
            return len;
        }

        self.cached_write_index = self.buffer.real_write_index.0.load(Ordering::Acquire);
        self.cached_write_index.wrapping_sub(self.read_index) & (S - 1)
    }

    #[inline]
    pub fn is_empty(&mut self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn pop(&mut self) -> T {
        loop {
            if let Some(value) = self.try_pop() {
                return value;
            }
            spin_loop();
        }
    }

    #[cfg_attr(feature = "profiling", inline(never))]
    #[cfg_attr(not(feature = "profiling"), inline(always))]
    pub fn try_pop(&mut self) -> Option<T> {
        if self.read_index == self.cached_write_index {
            self.cached_write_index = self.buffer.real_write_index.0.load(Ordering::Acquire);

            if self.read_index == self.cached_write_index {
                return None;
            }
        }
        let value =
            unsafe { (*self.buffer.ring.get_unchecked(self.read_index).get()).assume_init_read() };

        self.read_index = (self.read_index + 1) & (S - 1);

        self.buffer
            .real_read_index
            .0
            .store(self.read_index, Ordering::Release);

        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

    fn occupied_len<T, const S: usize>(buffer: &SPSCRBuffer<T, S>) -> usize
    where
        T: Send,
    {
        let write = buffer.real_write_index.0.load(Ordering::Acquire);
        let read = buffer.real_read_index.0.load(Ordering::Acquire);
        write.wrapping_sub(read) & S.wrapping_sub(1)
    }

    fn assert_state_from_consumer<T, const S: usize>(
        consumer: &SPSCConsumer<'_, T, S>,
        expected_len: usize,
    ) where
        T: Send,
    {
        let write = consumer.buffer.real_write_index.0.load(Ordering::Acquire);
        let read = consumer.buffer.real_read_index.0.load(Ordering::Acquire);
        assert!(write < S, "write index out of bounds: {write} >= {S}");
        assert!(read < S, "read index out of bounds: {read} >= {S}");

        let len = occupied_len(consumer.buffer);
        assert_eq!(len, expected_len, "unexpected occupancy");
        assert!(
            len <= S.saturating_sub(1),
            "occupancy exceeds ring capacity: {len} > {}",
            S.saturating_sub(1)
        );
    }

    #[test]
    fn read_from_empty_is_idempotent_and_keeps_internal_state() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for _ in 0..64 {
            assert_eq!(consumer.try_pop(), None);
            assert_state_from_consumer(&consumer, 0);
        }

        assert_eq!(producer.try_push(42), Ok(()));
        assert_eq!(consumer.try_pop(), Some(42));
        assert_state_from_consumer(&consumer, 0);
        assert_eq!(consumer.try_pop(), None);
        assert_state_from_consumer(&consumer, 0);
    }

    #[test]
    fn dropping_producer_does_not_invalidate_consumer() {
        let mut buffer = SPSCRBuffer::<u32, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for value in [11_u32, 22, 33] {
            assert_eq!(producer.try_push(value), Ok(()));
        }
        assert_state_from_consumer(&consumer, 3);

        drop(producer);
        assert_eq!(consumer.try_pop(), Some(11));
        assert_state_from_consumer(&consumer, 2);
        assert_eq!(consumer.try_pop(), Some(22));
        assert_state_from_consumer(&consumer, 1);
        assert_eq!(consumer.try_pop(), Some(33));
        assert_state_from_consumer(&consumer, 0);
        assert_eq!(consumer.try_pop(), None);
        assert_state_from_consumer(&consumer, 0);
    }
}
