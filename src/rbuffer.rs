use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use crossbeam_utils::atomic::AtomicCell;


pub struct Producer<T, const S: usize>
    where T: Copy + Default {

    rb: Arc<StackBuffer::<T, S>>,

}

impl<T, const S: usize> Producer<T, S> 
    where T: Copy + Default {

    pub fn try_push(&self, elem: T) -> Option<()>{


        let write_index = self.rb.write_index.load(Ordering::Acquire);
        let read_index = self.rb.read_index.load(Ordering::Acquire);

        if self.rb.is_full(read_index, write_index) {
            return None;
        }

        self.rb.write_index.fetch_update(Ordering::Release, Ordering::Relaxed, |i| (i + 1)%S);
        self.rb.rb[write_index].store(elem);
        Some(())
    }
}

pub struct Consumer<T, const S: usize>
    where T: Copy + Default {

    rb: Arc<StackBuffer::<T, S>>,
}


#[derive(Default)]
struct Slot<T> 
    where T: Copy + Default
{
    data: AtomicCell<T>,
    sequence: AtomicUsize,
}



pub(crate) struct StackBuffer<T, const S: usize> 
    where T: Copy + Default {
    rb: [Slot<T>; S],
    write_index: AtomicUsize,
    read_index: AtomicUsize,
}

impl<T, const S: usize> StackBuffer<T, S> 
    where T: Copy + Default {
    pub fn new() -> Self {
        Self {
            rb: std::array::from_fn::<Slot<T>, S, _>(|_| Slot::default()),
            write_index: AtomicUsize::default(),
            read_index: AtomicUsize::default(),
        }
    }

    pub fn is_full(&self, read_index: usize, write_index: usize) -> bool {
        
        (write_index + 1) % S == read_index
    }
}
