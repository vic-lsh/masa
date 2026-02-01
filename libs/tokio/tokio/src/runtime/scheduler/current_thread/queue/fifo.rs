use std::collections::VecDeque;

use super::{PopError, PushError, Queue, SchedFlavor};

/// A First-In-First-Out queue
#[derive(Debug)]
pub struct FifoQueue<T> {
    inner: VecDeque<T>,
    push_count: u64,
}

impl<T: 'static + Send + Sync> masa::Queue for FifoQueue<T> {
    fn queue_type() -> masa::QueueType {
        masa::QueueType::Fifo
    }
}

impl<T> Queue for FifoQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
            push_count: 0,
        }
    }

    fn new(_ty: Option<masa::QueueType>, cap: usize) -> Self {
        Self::with_capacity(cap)
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.inner.push_back(item);
        self.push_count += 1;


        // if self.push_count % 1000 == 0 {
        //     println!(
        //         "FifoQ: push_count {}, Current len {}, ratio {}",
        //         self.push_count,
        //         self.len(),
        //         self.push_count as f64 / self.len() as f64
        //     );
        // }
        Ok(())
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        self.inner.pop_front().ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.inner.capacity())
    }

    fn sched_flavor(&self) -> SchedFlavor {
        SchedFlavor::Fifo
    }
}
