use std::{
    collections::{BinaryHeap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{IntoSchedFlavor, PopError, PushError, Queue, SchedFlavor};
use crate::runtime::task::{Identifiable, Traceable};
use masa_core::Prioritize;

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

pub(crate) struct BinaryHeapQueue<T> {
    q: BinaryHeap<T>,
    infra_q: VecDeque<T>,
    push_count: u64,
    // reorder_count: u64,
}

impl<T: Ord + PartialOrd + Prioritize + Identifiable + Traceable> Queue for BinaryHeapQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            q: BinaryHeap::with_capacity(cap),
            infra_q: VecDeque::new(),
            push_count: 0,
        }
    }

    fn push(&mut self, mut item: Self::Item) -> Result<(), PushError<Self::Item>> {
        item.timer().set_enqueue_time();
        // let id = item.id();
        
        if item.priority().value() == 0 {
            self.infra_q.push_back(item);
        } else {
            self.q.push(item);
        }
        
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
        if let Some(mut e) = self.infra_q.pop_front() {
            e.timer().record_queue_lat();
            return Ok(e);
        }
        
        self.q.pop().ok_or(PopError::Empty).map(|mut e| {
            e.timer().record_queue_lat();
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
        self.q.len() + self.infra_q.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.q.capacity() + self.infra_q.capacity())
    }
}

impl<T: Ord> Default for BinaryHeapQueue<T> {
    fn default() -> Self {
        Self {
            q: BinaryHeap::new(),
            infra_q: VecDeque::new(),
            push_count: 0,
        }
    }
}

impl<T> IntoSchedFlavor for BinaryHeapQueue<T> {
    fn into_sched_flavor() -> SchedFlavor {
        SchedFlavor::Prio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prio_queue_sched_flavor() {
        assert_eq!(
            BinaryHeapQueue::<u64>::into_sched_flavor(),
            SchedFlavor::Prio
        );
    }
}
