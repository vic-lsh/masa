/// Priority hint of a future. Smaller priorities mean higher priority. The default priority is `infra`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PriorityHint(u64);

impl PriorityHint {
    /// Create a priority hint for infrastructure tasks, which has the highest priority.
    pub fn infra() -> Self {
        Self(0)
    }

    /// Create a new priority hint.
    pub fn new(hint: u64) -> Self {
        Self(hint)
    }

    /// Get the priority hint.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl PartialOrd for PriorityHint {
    /// Flip the order for a min-heap.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}

impl Ord for PriorityHint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// Structs that have a notion of priority hint should implement this trait.
pub trait Prioritize {
    /// Return the priority hint associated with this struct.
    fn priority(&self) -> PriorityHint;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deadline_hint_partial_eq() {
        assert_eq!(PriorityHint::new(1), PriorityHint::new(1));
        assert_ne!(PriorityHint::new(1), PriorityHint::new(2));
        assert_ne!(PriorityHint::new(1), PriorityHint::infra());
        assert_ne!(PriorityHint::infra(), PriorityHint::new(1));
        assert_eq!(PriorityHint::infra(), PriorityHint::infra());
    }

    #[test]
    fn test_deadline_hint_partial_ord() {
        assert_eq!(
            PriorityHint::new(1).partial_cmp(&PriorityHint::new(1)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            PriorityHint::new(1).partial_cmp(&PriorityHint::new(2)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            PriorityHint::new(2).partial_cmp(&PriorityHint::new(1)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            PriorityHint::infra().partial_cmp(&PriorityHint::new(1)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            PriorityHint::new(1).partial_cmp(&PriorityHint::infra()),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            PriorityHint::infra().partial_cmp(&PriorityHint::infra()),
            Some(std::cmp::Ordering::Equal)
        );
    }
}
