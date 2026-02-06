use std::collections::VecDeque;

use super::{IntoSchedFlavor, PopError, PushError, Queue, SchedFlavor};

#[allow(dead_code)]
pub(crate) struct FifoQueue<T> {
    inner: VecDeque<T>,
    push_count: u64,
}

impl<T> Queue for FifoQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
            push_count: 0,
        }
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
}

impl<T> IntoSchedFlavor for FifoQueue<T> {
    fn into_sched_flavor() -> SchedFlavor {
        SchedFlavor::Fifo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_queue_sched_flavor() {
        assert_eq!(FifoQueue::<u64>::into_sched_flavor(), SchedFlavor::Fifo);
    }
}
