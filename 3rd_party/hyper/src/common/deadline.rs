/// Deadline hint of a future. Earlier deadlines mean higher priority. The default priority is `infra`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DeadlineHint(u64);

impl DeadlineHint {
    /// Create a deadline hint for infrastructure tasks, which has the highest priority.
    pub fn infra() -> Self {
        Self(0)
    }

    /// Create a new deadline hint.
    pub fn new(hint: u64) -> Self {
        Self(hint)
    }
}

impl PartialOrd for DeadlineHint {
    /// Flip the order for a min-heap.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}

impl Ord for DeadlineHint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deadline_hint_partial_eq() {
        assert_eq!(DeadlineHint::new(1), DeadlineHint::new(1));
        assert_ne!(DeadlineHint::new(1), DeadlineHint::new(2));
        assert_ne!(DeadlineHint::new(1), DeadlineHint::infra());
        assert_ne!(DeadlineHint::infra(), DeadlineHint::new(1));
        assert_eq!(DeadlineHint::infra(), DeadlineHint::infra());
    }

    #[test]
    fn test_deadline_hint_partial_ord() {
        assert_eq!(
            DeadlineHint::new(1).partial_cmp(&DeadlineHint::new(1)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            DeadlineHint::new(1).partial_cmp(&DeadlineHint::new(2)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            DeadlineHint::new(2).partial_cmp(&DeadlineHint::new(1)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            DeadlineHint::infra().partial_cmp(&DeadlineHint::new(1)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            DeadlineHint::new(1).partial_cmp(&DeadlineHint::infra()),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            DeadlineHint::infra().partial_cmp(&DeadlineHint::infra()),
            Some(std::cmp::Ordering::Equal)
        );
    }
}
