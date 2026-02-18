use crate::cache_padding::CachePadded;
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicUsize;

pub struct Slot<T>
where
    T: Send,
{
    pub value: MaybeUninit<T>,
    pub sequence: AtomicUsize,
}

impl<T> Default for Slot<T>
where
    T: Send,
{
    fn default() -> Self {
        Self {
            value: MaybeUninit::<T>::uninit(),
            sequence: AtomicUsize::new(0),
        }
    }
}

impl<T> Slot<T>
where
    T: Send,
{
    fn new(i: usize) -> Self {
        Self {
            value: MaybeUninit::<T>::uninit(),
            sequence: AtomicUsize::new(i),
        }
    }
}

pub struct MPSCRBuffer<T, const S: usize>
where
    T: Send,
{
    pub(super) real_write_index: CachePadded<AtomicUsize>,
    pub(super) ring: [UnsafeCell<Slot<T>>; S],
}

unsafe impl<T: Send, const S: usize> Sync for MPSCRBuffer<T, S> {}

impl<T, const S: usize> Drop for MPSCRBuffer<T, S>
where
    T: Send,
{
    fn drop(&mut self) {
        todo!()
        // let mut read = self.real_read_index.0.load(Ordering::Relaxed);

        // let write = self.real_write_index.0.load(Ordering::Relaxed);

        // while read != write {
        //     unsafe {
        //         (*self.ring[read].get()).assume_init_drop();
        //     }

        //     read = (read + 1) & (S - 1);
        // }
    }
}
impl<T, const S: usize> Default for MPSCRBuffer<T, S>
where
    T: Send,
{
    fn default() -> Self {
        assert!(S.is_power_of_two());
        assert!(S > 1);
        Self {
            real_write_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<Slot<T>>, S, _>(|i| {
                UnsafeCell::new(Slot::<T>::new(i))
            }),
        }
    }
}

impl<T, const S: usize> MPSCRBuffer<T, S>
where
    T: Send,
{
    pub fn new() -> Self {
        assert!(S.is_power_of_two());
        assert!(S > 1);
        Self {
            real_write_index: CachePadded(AtomicUsize::new(0)),
            ring: core::array::from_fn::<UnsafeCell<Slot<T>>, S, _>(|i| {
                UnsafeCell::new(Slot::<T>::new(i))
            }),
        }
    }

    pub fn split(&mut self) {
        // let write = self.real_write_index.0.load(Ordering::Relaxed);
        // let read = self.real_read_index.0.load(Ordering::Relaxed);
        todo!()
    }
}
