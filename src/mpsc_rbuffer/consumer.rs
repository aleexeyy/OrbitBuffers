use core::{hint::spin_loop, sync::atomic::Ordering};

use super::MPSCRBuffer;

pub struct MPSCConsumer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a MPSCRBuffer<T, S>,
    pub(super) position: usize,
}

// implement drop to silence clippy warnings

impl<'a, T, const S: usize> Drop for MPSCConsumer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> MPSCConsumer<'a, T, S>
where
    T: Send,
{
    #[inline]
    pub const fn capacity(&self) -> usize {
        S - 1
    }

    #[inline]
    pub fn len(&mut self) -> usize {
        let writer_pos = self.buffer.real_write_index.0.load(Ordering::Relaxed);

        let writer_index = writer_pos & (S - 1);

        writer_index.wrapping_sub(self.position) & (S - 1)
    }

    #[inline]
    pub fn is_empty(&mut self) -> bool {
        self.len() == 0
    }

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
        let index = self.position & (S - 1);

        let slot = unsafe { &mut *self.buffer.ring.get_unchecked(index).get() };

        let sequence = slot.sequence.load(Ordering::Acquire);
        let data: T;

        let new_position = self.position.wrapping_add(1);

        if sequence == new_position {
            data = unsafe { slot.value.assume_init_read() };

            slot.sequence
                .store(self.position.wrapping_add(S), Ordering::Release);
            self.position = new_position;

            unsafe {
                *self.buffer.real_read_index.0.get() = new_position;
            };

            return Some(data);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

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

    #[test]
    fn single_producer_single_consumer_basic_flow() {
        let mut buffer = MPSCRBuffer::<u64, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for _ in 0..32 {
            assert_eq!(consumer.try_pop(), None);
        }

        for value in 0_u64..8 {
            assert!(producer.push(value).is_ok());
            assert_distance_within_capacity(consumer.buffer);
        }

        for expected in 0_u64..8 {
            assert_eq!(consumer.pop(), expected);
            assert_distance_within_capacity(consumer.buffer);
        }

        assert_eq!(consumer.try_pop(), None);
        let (write, read) = positions(consumer.buffer);
        assert_eq!(write, read);
    }

    #[test]
    fn push_until_full_then_pop_until_empty() {
        let mut buffer = MPSCRBuffer::<usize, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for value in 0..8 {
            assert!(producer.push(value).is_ok());
        }
        assert_distance_within_capacity(consumer.buffer);

        for expected in 0..8 {
            assert_eq!(consumer.pop(), expected);
        }

        assert_eq!(consumer.try_pop(), None);
        let (write, read) = positions(consumer.buffer);
        assert_eq!(write, read);
    }
}
