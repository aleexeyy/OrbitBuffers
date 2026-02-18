use core::{hint::spin_loop, sync::atomic::Ordering};

use super::MPSCRBuffer;

pub struct MultiConsumer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a MPSCRBuffer<T, S>,
    pub(super) position: usize,
}

// implement drop to silence clippy warnings

impl<'a, T, const S: usize> Drop for MultiConsumer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> MultiConsumer<'a, T, S>
where
    T: Send,
{
    #[inline]
    pub const fn capacity(&self) -> usize {
        S - 1
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

        if sequence == self.position.wrapping_add(1) {
            data = unsafe { slot.value.assume_init_read() };

            slot.sequence
                .store(self.position.wrapping_add(S), Ordering::Release);
            self.position = self.position.wrapping_add(1);

            return Some(data);
        }

        None
    }
}
