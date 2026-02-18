use super::rbuffer::SPSCRBuffer;
use core::hint::spin_loop;
use core::sync::atomic::Ordering;

pub struct SingleProducer<'a, T, const S: usize>
where
    T: Send,
{
    pub(super) buffer: &'a SPSCRBuffer<T, S>,
    pub(super) write_index: usize,
    pub(super) cached_read_index: usize,
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
