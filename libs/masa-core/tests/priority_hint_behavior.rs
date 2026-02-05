use std::collections::BinaryHeap;

use masa_core::PriorityHint;

#[test]
fn binary_heap_pops_infra_first() {
    let mut heap = BinaryHeap::new();
    heap.push(PriorityHint::new(500_000));
    heap.push(PriorityHint::new(250_000));
    heap.push(PriorityHint::infra());

    assert_eq!(heap.pop(), Some(PriorityHint::infra()));
    assert_eq!(heap.pop(), Some(PriorityHint::new(250_000)));
    assert_eq!(heap.pop(), Some(PriorityHint::new(500_000)));
    assert!(heap.is_empty());
}

#[test]
fn larger_deadline_means_lower_priority() {
    let mut heap = BinaryHeap::new();
    for offset in 0..5_u64 {
        heap.push(PriorityHint::new(1_000_000 + offset * 100));
    }

    let mut previous = PriorityHint::infra();
    while let Some(next) = heap.pop() {
        assert!(previous >= next, "heap returned out-of-order priority" );
        previous = next;
    }
}
