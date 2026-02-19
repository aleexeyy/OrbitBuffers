use core::{hint::spin_loop, sync::atomic::Ordering};

use super::MPSCRBuffer;

pub struct MPSCProducer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a MPSCRBuffer<T, S>,
}

impl<'a, T, const S: usize> Clone for MPSCProducer<'a, T, S>
where
    T: Send,
{
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer,
        }
    }
}

// implement drop to silence clippy warnings

impl<'a, T, const S: usize> Drop for MPSCProducer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> MPSCProducer<'a, T, S>
where
    T: Send,
{
    #[inline]
    pub const fn capacity(&self) -> usize {
        S - 1
    }

    #[cfg_attr(feature = "profiling", inline(never))]
    #[cfg_attr(not(feature = "profiling"), inline(always))]
    pub fn push(&mut self, data: T) -> Result<(), T> {
        let pos = self
            .buffer
            .real_write_index
            .0
            .fetch_add(1, Ordering::Relaxed);

        let index = pos & (S - 1);

        let slot = unsafe { &mut *self.buffer.ring.get_unchecked(index).get() };

        loop {
            let slot_sequence = slot.sequence.load(Ordering::Acquire);

            if slot_sequence == pos {
                slot.value.write(data);
                slot.sequence.store(pos.wrapping_add(1), Ordering::Release);
                return Ok(());
            }

            spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn blocked_push_completes_after_consumer_makes_space() {
        let mut buffer = MPSCRBuffer::<usize, 8>::new();
        let (mut producer, mut consumer) = buffer.split();

        for value in 0..8 {
            assert!(producer.push(value).is_ok());
        }

        let start = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let start_push = start.clone();
            let mut blocked_producer = MPSCProducer {
                buffer: producer.buffer,
            };
            let push_handle = scope.spawn(move || {
                start_push.wait();
                assert!(blocked_producer.push(999).is_ok());
            });

            start.wait();
            assert_eq!(consumer.pop(), 0);
            push_handle.join().expect("producer thread panicked");
        });

        for expected in 1..8 {
            assert_eq!(consumer.pop(), expected);
        }
        assert_eq!(consumer.pop(), 999);
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn rapid_producer_handle_churn() {
        let mut buffer = MPSCRBuffer::<usize, 64>::new();
        let (first, mut consumer) = buffer.split();
        let shared = first.buffer;
        drop(first);

        for value in 0..10_000 {
            let mut producer = MPSCProducer { buffer: shared };
            assert!(producer.push(value).is_ok());
            assert_eq!(consumer.pop(), value);
        }
        assert_eq!(consumer.try_pop(), None);
    }
}
