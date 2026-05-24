/// Neutral runtime task priority.
///
/// Smaller values have higher priority. `TaskPriority::infra()` is priority 0
/// and is reserved for runtime and transport infrastructure work.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TaskPriority(u64);

impl TaskPriority {
    /// Create the infrastructure task priority.
    pub fn infra() -> Self {
        Self(0)
    }

    /// Create a new task priority.
    pub fn new(priority: u64) -> Self {
        Self(priority)
    }

    /// Return the raw priority value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::infra()
    }
}

impl PartialOrd for TaskPriority {
    /// Flip the order so smaller priority values win in `BinaryHeap`.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}

impl Ord for TaskPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

#[cfg_attr(not(feature = "sched_prio"), allow(dead_code))]
pub(crate) trait TaskPrioritize {
    fn priority(&self) -> TaskPriority;
}

#[cfg(test)]
mod tests {
    use super::TaskPriority;
    use std::collections::BinaryHeap;

    #[test]
    fn infra_is_default_priority_zero() {
        assert_eq!(TaskPriority::infra(), TaskPriority::default());
        assert_eq!(TaskPriority::infra().value(), 0);
    }

    #[test]
    fn smaller_priority_values_sort_first() {
        assert_eq!(
            TaskPriority::new(1).partial_cmp(&TaskPriority::new(2)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            TaskPriority::new(2).partial_cmp(&TaskPriority::new(1)),
            Some(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn binary_heap_pops_lowest_priority_value_first() {
        let mut heap = BinaryHeap::new();
        heap.push(TaskPriority::new(500_000));
        heap.push(TaskPriority::new(250_000));
        heap.push(TaskPriority::infra());

        assert_eq!(heap.pop(), Some(TaskPriority::infra()));
        assert_eq!(heap.pop(), Some(TaskPriority::new(250_000)));
        assert_eq!(heap.pop(), Some(TaskPriority::new(500_000)));
    }
}
