use std::collections::VecDeque;

use super::{PopError, PushError, Queue};

pub(crate) struct FifoQueue<T> {
    inner: VecDeque<T>,
}

impl<T> FifoQueue<T> {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
        }
    }
}

impl<T> Queue for FifoQueue<T> {
    type Item = T;

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.inner.push_back(item);
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
