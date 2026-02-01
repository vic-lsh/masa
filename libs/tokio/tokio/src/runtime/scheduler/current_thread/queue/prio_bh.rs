use std::{
    collections::BinaryHeap,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{PopError, PushError, Queue, SchedFlavor};
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

#[allow(dead_code)]
fn ms_since_init(value: u64) -> u64 {
    (value - *INIT) / 1000
}

/// A Binary Heap based Priority Queue
#[derive(Debug)]
pub struct BinaryHeapQueue<T> {
    q: BinaryHeap<T>,
    push_count: u64,
    // reorder_count: u64,
}

impl<T: 'static + Send + Sync> masa::Queue for BinaryHeapQueue<T> {
    fn queue_type() -> masa::QueueType {
        masa::QueueType::Prio
    }
}

impl<T: Ord + PartialOrd + Prioritize + Identifiable> Queue for BinaryHeapQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            q: BinaryHeap::with_capacity(cap),
            push_count: 0,
        }
    }

    fn new(_ty: Option<masa::QueueType>, cap: usize) -> Self {
        Self::with_capacity(cap)
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        // let id = item.id();
        self.q.push(item);
        self.push_count += 1;

        // Get slice of binary heap and find the index of the newly added element
        // if self.push_count % 1000 == 0 {
        //     println!(
        //         "PrioBHQ: push_count {}, Current len {}, ratio {}",
        //         self.push_count,
        //         self.len(),
        //         self.push_count as f64 / self.len() as f64
        //     );
        // }

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

    fn sched_flavor(&self) -> SchedFlavor {
        SchedFlavor::Prio
    }
}

impl<T: Ord> Default for BinaryHeapQueue<T> {
    fn default() -> Self {
        Self {
            q: BinaryHeap::new(),
            push_count: 0,
        }
    }
}
