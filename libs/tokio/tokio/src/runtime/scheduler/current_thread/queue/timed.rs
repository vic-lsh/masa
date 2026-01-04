use crate::runtime::{
    scheduler::current_thread::{
        queue::{PopError, PushError, Queue},
        Handle,
    },
    task,
};
use std::sync::Arc;

/// Timed Queue
pub(crate) struct TimedQueue<Q> {
    inner: Q,
}

impl<Q: Queue<Item = Notified>> Queue for TimedQueue<Q> {
    type Item = Q::Item;

    fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Q::with_capacity(cap),
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        item.timer().set_enqueue_time();
        self.inner.push(item)
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        let item = self.inner.pop()?;
        item.timer().record_queue_lat();
        Ok(item)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }
}
