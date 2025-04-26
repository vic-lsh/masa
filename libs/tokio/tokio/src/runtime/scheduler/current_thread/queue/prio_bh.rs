use std::{
    collections::BinaryHeap,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{PopError, PushError, Queue};
use crate::runtime::task::Identifiable;
use masa::Prioritize;

#[allow(dead_code)]
static INIT: std::sync::LazyLock<u64> = std::sync::LazyLock::new(time_now);

#[inline]
fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

fn ms_since_init(value: u64) -> u64 {
    (value - *INIT) / 1000
}

pub(crate) struct BinaryHeapQueue<T> {
    q: BinaryHeap<T>,
}

impl<T: Ord + PartialOrd + Prioritize + Identifiable> Queue for BinaryHeapQueue<T> {
    type Item = T;

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.q.push(item);
        Ok(())
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        // [TODO] Do something with a task if it is already expired.
        self.q.pop().ok_or(PopError::Empty).map(|e| {
            // for debugging
            // let prio = e.priority();
            // let ms_since_launch = if prio.value() == 0 {
            //     0
            // } else {
            //     ms_since_init(prio.value())
            // };
            // println!("Popped elem id {} prio {}", e.id(), ms_since_launch);
            e
        })
    }

    fn len(&self) -> usize {
        self.q.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.q.capacity())
    }
}

impl<T: Ord> Default for BinaryHeapQueue<T> {
    fn default() -> Self {
        Self {
            q: BinaryHeap::new(),
        }
    }
}

impl<T: Ord> BinaryHeapQueue<T> {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            q: BinaryHeap::with_capacity(cap),
        }
    }
}
