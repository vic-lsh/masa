//! Replay tests that pin the pop order of every local run queue.
//!
//! Each script is a sequence of pushes and pops. The expected pop order and
//! queue length after every step were recorded from the queues as implemented
//! before the `Meta` refactor and must not change when that refactor lands.
//! A pop on an empty queue is recorded as `-1`.

use crate::{Meta, RunQueue, TaskView};

#[derive(Clone, Copy)]
enum Op {
    /// Push a task with `(id, priority)`.
    Push(u64, u64),
    Pop,
}

use Op::{Pop, Push};

/// Runs `script` against a fresh queue; returns the popped ids and the queue
/// length after every step.
fn replay<Q: RunQueue<u64>>(cap: usize, script: &[Op]) -> (Vec<i64>, Vec<usize>) {
    let mut q = Q::with_capacity(cap);
    let t0 = std::time::Instant::now();
    let mut popped = Vec::new();
    let mut lens = Vec::new();
    for op in script {
        match *op {
            Push(id, prio) => q.push(id, &TaskView::new(id, &Meta::new(prio), t0, t0, 0)),
            Pop => popped.push(q.pop().map_or(-1, |id| id as i64)),
        }
        lens.push(q.len());
    }
    (popped, lens)
}

const TIES: &[Op] = &[
    Push(1, 10),
    Push(2, 10),
    Push(3, 10),
    Push(4, 10),
    Push(5, 10),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

const MIXED_WITH_INFRA: &[Op] = &[
    Push(1, 30),
    Push(2, 0),
    Push(3, 20),
    Push(4, 0),
    Push(5, 20),
    Push(6, 10),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

const INTERLEAVED: &[Op] = &[
    Push(1, 50),
    Push(2, 40),
    Pop,
    Push(3, 30),
    Push(4, 0),
    Pop,
    Pop,
    Push(5, 45),
    Pop,
    Pop,
    Pop,
    Pop,
];

const ONLY_INFRA: &[Op] = &[
    Push(1, 0),
    Push(2, 0),
    Push(3, 0),
    Pop,
    Push(4, 0),
    Pop,
    Pop,
    Pop,
    Pop,
];

const EMPTY_POPS: &[Op] = &[Pop, Pop, Push(1, 5), Pop, Pop, Push(2, 7), Pop];

/// More tasks than the round-robin window (6) with distinct priorities, pushed
/// in an order that differs from priority order.
const RR_DISTINCT: &[Op] = &[
    Push(1, 80),
    Push(2, 20),
    Push(3, 70),
    Push(4, 30),
    Push(5, 60),
    Push(6, 40),
    Push(7, 50),
    Push(8, 10),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

/// Pushes after the round-robin window is filled: one that evicts, one that
/// does not, and a tie with the window's worst element.
const RR_EVICTION: &[Op] = &[
    Push(1, 10),
    Push(2, 20),
    Push(3, 30),
    Push(4, 40),
    Push(5, 50),
    Push(6, 60),
    Push(7, 70),
    Pop,
    Push(8, 5),
    Push(9, 80),
    Push(10, 60),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

const RR_TIES: &[Op] = &[
    Push(1, 10),
    Push(2, 10),
    Push(3, 10),
    Push(4, 10),
    Push(5, 10),
    Push(6, 10),
    Push(7, 10),
    Push(8, 10),
    Push(9, 10),
    Pop,
    Push(10, 10),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

const RR_INFRA_INTERLEAVED: &[Op] = &[
    Push(1, 30),
    Push(2, 20),
    Pop,
    Push(3, 0),
    Push(4, 10),
    Push(5, 0),
    Pop,
    Pop,
    Push(6, 0),
    Pop,
    Pop,
    Pop,
    Pop,
    Pop,
];

/// Scripts with the queue length expected after every step. Lengths do not
/// depend on the queue implementation.
const CASES: &[(&str, &[Op], &[usize])] = &[
    ("TIES", TIES, &[1, 2, 3, 4, 5, 4, 3, 2, 1, 0, 0]),
    (
        "MIXED_WITH_INFRA",
        MIXED_WITH_INFRA,
        &[1, 2, 3, 4, 5, 6, 5, 4, 3, 2, 1, 0, 0],
    ),
    (
        "INTERLEAVED",
        INTERLEAVED,
        &[1, 2, 1, 2, 3, 2, 1, 2, 1, 0, 0, 0],
    ),
    ("ONLY_INFRA", ONLY_INFRA, &[1, 2, 3, 2, 3, 2, 1, 0, 0]),
    ("EMPTY_POPS", EMPTY_POPS, &[0, 0, 1, 0, 0, 1, 0]),
    (
        "RR_DISTINCT",
        RR_DISTINCT,
        &[1, 2, 3, 4, 5, 6, 7, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0],
    ),
    (
        "RR_EVICTION",
        RR_EVICTION,
        &[
            1, 2, 3, 4, 5, 6, 7, 6, 7, 8, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0,
        ],
    ),
    (
        "RR_TIES",
        RR_TIES,
        &[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 8, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0,
        ],
    ),
    (
        "RR_INFRA_INTERLEAVED",
        RR_INFRA_INTERLEAVED,
        &[1, 2, 1, 2, 3, 4, 3, 2, 3, 2, 1, 0, 0, 0],
    ),
];

/// Capacities the queues are constructed with: zero, one, and an amount larger
/// than any script. The pop order must not depend on the capacity hint.
const CAPS: [usize; 3] = [0, 1, 64];

/// Replays every case against `Q` and compares with `expected_pops`, which is
/// aligned with `CASES`.
fn check<Q: RunQueue<u64>>(expected_pops: &[&[i64]]) {
    assert_eq!(expected_pops.len(), CASES.len());
    for ((name, script, lens), pops) in CASES.iter().zip(expected_pops) {
        for cap in CAPS {
            let (p, l) = replay::<Q>(cap, script);
            assert_eq!(&p, pops, "{name}: pop order at capacity {cap}");
            assert_eq!(&l, lens, "{name}: queue length trace at capacity {cap}");
        }
    }
}

const FIFO_POPS: &[&[i64]] = &[
    &[1, 2, 3, 4, 5, -1],
    &[1, 2, 3, 4, 5, 6, -1],
    &[1, 2, 3, 4, 5, -1, -1],
    &[1, 2, 3, 4, -1],
    &[-1, -1, 1, -1, 2],
    &[1, 2, 3, 4, 5, 6, 7, 8, -1],
    &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, -1],
    &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, -1],
    &[1, 2, 3, 4, 5, 6, -1, -1],
];

const PRIO_HEAP_POPS: &[&[i64]] = &[
    &[1, 3, 5, 2, 4, -1],
    &[2, 4, 6, 5, 3, 1, -1],
    &[2, 4, 3, 5, 1, -1, -1],
    &[1, 2, 3, 4, -1],
    &[-1, -1, 1, -1, 2],
    &[8, 2, 4, 6, 7, 5, 3, 1, -1],
    &[1, 8, 2, 3, 4, 5, 6, 10, 7, 9, -1],
    &[1, 3, 7, 9, 10, 6, 8, 5, 2, 4, -1],
    &[2, 3, 5, 6, 4, 1, -1, -1],
];

const TAILCLIPPER_POPS_WITHOUT_INFRA_QUEUE: &[&[i64]] = &[
    &[1, 3, 5, 2, 4, -1],
    &[2, 4, 6, 3, 5, 1, -1],
    &[2, 1, 3, 4, 5, -1, -1],
    &[1, 2, 3, 4, -1],
    &[-1, -1, 1, -1, 2],
    &[8, 2, 4, 6, 7, 5, 3, 1, -1],
    &[1, 2, 3, 4, 5, 6, 8, 10, 7, 9, -1],
    &[1, 3, 7, 9, 6, 8, 5, 4, 2, 10, -1],
    &[2, 1, 3, 4, 5, 6, -1, -1],
];

const TAILCLIPPER_POPS_WITH_INFRA_QUEUE: &[&[i64]] = &[
    &[1, 3, 5, 2, 4, -1],
    &[2, 4, 6, 5, 3, 1, -1],
    &[2, 4, 1, 3, 5, -1, -1],
    &[1, 2, 3, 4, -1],
    &[-1, -1, 1, -1, 2],
    &[8, 2, 4, 6, 7, 5, 3, 1, -1],
    &[1, 2, 3, 4, 5, 6, 8, 10, 7, 9, -1],
    &[1, 3, 7, 9, 6, 8, 5, 4, 2, 10, -1],
    &[2, 3, 5, 6, 1, 4, -1, -1],
];

#[test]
fn fifo_replay_scripts() {
    check::<crate::builtin::fifo::FifoQueue<u64>>(FIFO_POPS);
}

#[test]
fn prio_heap_replay_scripts() {
    check::<crate::builtin::prio_heap::BinaryHeapQueue<u64>>(PRIO_HEAP_POPS);
}

#[test]
fn tailclipper_replay_scripts_without_infra_queue() {
    check::<crate::builtin::tailclipper::BinaryHeapRoundRobinQueue<u64, false>>(
        TAILCLIPPER_POPS_WITHOUT_INFRA_QUEUE,
    );
}

#[test]
fn tailclipper_replay_scripts_with_infra_queue() {
    check::<crate::builtin::tailclipper::BinaryHeapRoundRobinQueue<u64, true>>(
        TAILCLIPPER_POPS_WITH_INFRA_QUEUE,
    );
}

/// `custom::Queue` starts as a copy of the priority heap.
#[test]
fn custom_replay_scripts() {
    check::<crate::custom::Queue<u64>>(PRIO_HEAP_POPS);
}

/// The queue the active Cargo features select behaves as that queue's script
/// expects.
#[test]
fn selected_queue_replay_scripts() {
    #[cfg(feature = "sched_custom")]
    let expected = PRIO_HEAP_POPS;
    #[cfg(all(
        feature = "sched_prio",
        feature = "tailclipper",
        not(feature = "sched_custom")
    ))]
    let expected = TAILCLIPPER_POPS_WITHOUT_INFRA_QUEUE;
    #[cfg(all(
        feature = "sched_prio",
        not(feature = "tailclipper"),
        not(feature = "sched_custom")
    ))]
    let expected = PRIO_HEAP_POPS;
    #[cfg(not(any(feature = "sched_prio", feature = "sched_custom")))]
    let expected = FIFO_POPS;

    check::<crate::SelectedQueue<u64>>(expected);
}
