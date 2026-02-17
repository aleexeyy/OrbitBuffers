use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use crossbeam_utils::atomic::AtomicCell;


pub struct SPSCRBuffer<T, const S: usize>
where T: Copy + Default {

    buffer: [AtomicCell<T>; S],
    real_write_index: AtomicUsize,
    real_read_index: AtomicUsize,
}

pub struct SingleProducer<T, const S: usize> where T: Copy + Default  {
    buffer: Arc<SPSCRBuffer<T, S>>,
    local_write_index: usize,
    local_read_index: usize,
}

impl <T, const S: usize> SingleProducer<T, S>
where T: Copy + Default {
    



    pub fn try_push(&mut self, data: T) -> Option<()> {


        if self.local_read_index == self.local_write_index {

            self.local_read_index = self.buffer.real_read_index.load(Ordering::Acquire);

            if self.local_read_index == self.local_write_index {

                return None;
            }

        }


        self.buffer.buffer[self.local_write_index].store(data);
        self.buffer.real_write_index.fetch_add(1, Ordering::Release);

        self.local_write_index += 1;
        

        Some(())
    }
}




pub struct SingleConsumer<T, const S: usize> where T: Copy + Default  {
    buffer: Arc<SPSCRBuffer<T, S>>,
    local_write_index: usize,
    local_read_index: usize,
}


impl <T, const S: usize> SPSCRBuffer<T, S>
where T: Copy + Default {

    pub fn new() -> Self {
        Self {
            buffer: std::array::from_fn::<AtomicCell<T>, S, _>(|_| AtomicCell::new(T::default())),
            real_write_index: AtomicUsize::default(),
            real_read_index: AtomicUsize::default(),
        }
    }


    pub fn create() -> (SingleProducer<T, S>, SingleConsumer<T, S>) {
        let shared = Arc::new(Self::new());
        (
            SingleProducer {
                buffer: shared.clone(),
                local_write_index: 0,
                local_read_index: 0,
            },
            SingleConsumer {
                buffer: shared,
                local_write_index: 0,
                local_read_index: 0,
            },
        )
    }

}


// TODO: the ring buffers supposed to implement: SingleProducer, SingleConsumer, MultipleProducer, MultipleConsumer traits and overall custom Buffer trait, and based on them create generic Consumers and Producers