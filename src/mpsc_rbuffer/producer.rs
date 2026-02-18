use core::{hint::spin_loop, sync::atomic::Ordering};

use super::MPSCRBuffer;

pub struct MultiProducer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a MPSCRBuffer<T, S>,
}

// implement drop to silence clippy warnings

impl<'a, T, const S: usize> Drop for MultiProducer<'a, T, S>
where
    T: Send,
{
    fn drop(&mut self) {}
}

impl<'a, T, const S: usize> MultiProducer<'a, T, S>
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

        // Err(data)
    }
}
