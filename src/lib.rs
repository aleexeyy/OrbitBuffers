#![cfg_attr(not(feature = "std"), no_std)]
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;
use core::sync::atomic::{AtomicBool, AtomicUsize};

pub struct SingleProducer<'a, T, const S: usize>
where
    T: Send,
{
    buffer: &'a SPSCRBuffer<T, S>,
    write_index: usize,
    cached_read_index: usize,
}

impl<'a, T, const S: usize> SingleProducer<'a, T, S>
where
    T: Send,
{
    pub fn try_push(&mut self, data: T) -> Option<()> {
        let next_write_index = (self.write_index + 1) & (S - 1);

        //check wether buffer is full and update cache if necessary
        if next_write_index == self.cached_read_index {
            self.cached_read_index = self.buffer.real_read_index.load(Ordering::Acquire);

            if next_write_index == self.cached_read_index {
                return None;
            }
        }

        // safety: the old value is dropped by consumer
        let _ = unsafe { (*self.buffer.ring[self.write_index].get()).write(data) };

        self.write_index = next_write_index;

        self.buffer
            .real_write_index
            .store(self.write_index, Ordering::Release);

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

impl<'a, T, const S: usize> SingleConsumer<'a, T, S>
where
    T: Send,
{
    pub fn try_read(&mut self) -> Option<T> {
        if self.read_index == self.cached_write_index {
            self.cached_write_index = self.buffer.real_write_index.load(Ordering::Acquire);

            if self.read_index == self.cached_write_index {
                return None;
            }
        }
        let value = unsafe { (*self.buffer.ring[self.read_index].get()).assume_init_read() };

        self.read_index = (self.read_index + 1) & (S - 1);

        self.buffer
            .real_read_index
            .store(self.read_index, Ordering::Release);

        Some(value)
    }
}

pub struct SPSCRBuffer<T, const S: usize>
where
    T: Send,
{
    ring: [UnsafeCell<MaybeUninit<T>>; S],
    real_write_index: AtomicUsize,
    real_read_index: AtomicUsize,
}

unsafe impl<T: Send, const S: usize> Sync for SPSCRBuffer<T, S> {}

impl<T, const S: usize> Drop for SPSCRBuffer<T, S>
where
    T: Send,
{
    fn drop(&mut self) {
        let mut read = self.real_read_index.load(Ordering::Relaxed);

        let write = self.real_write_index.load(Ordering::Relaxed);

        while read != write {
            unsafe {
                (*self.ring[read].get()).assume_init_drop();
            }

            read = (read + 1) & (S - 1);
        }
    }
}

impl<'a, T, const S: usize> SPSCRBuffer<T, S>
where
    T: Send,
{
    pub fn new() -> Self {
        assert!(S.is_power_of_two());
        Self {
            ring: core::array::from_fn::<UnsafeCell<MaybeUninit<T>>, S, _>(|_| {
                UnsafeCell::new(MaybeUninit::<T>::uninit())
            }),
            real_write_index: AtomicUsize::new(0),
            real_read_index: AtomicUsize::new(0),
        }
    }

    pub fn split(&mut self) -> Option<(SingleProducer<'_, T, S>, SingleConsumer<'_, T, S>)> {
        Some((
            SingleProducer {
                buffer: self,
                write_index: 0,
                cached_read_index: 0,
            },
            SingleConsumer {
                buffer: self,
                cached_write_index: 0,
                read_index: 0,
            },
        ))
    }
}
