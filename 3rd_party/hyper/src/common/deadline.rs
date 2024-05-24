/// Deadline hint of a future. Earlier deadlines mean higher priority. The default priority is `Background`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadlineHint {
    /// A deadline value.
    Some(u32),
    /// Default priority.
    Background,
}

impl Default for DeadlineHint {
    fn default() -> Self {
        DeadlineHint::Background
    }
}

impl PartialOrd for DeadlineHint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (DeadlineHint::Some(a), DeadlineHint::Some(b)) => a.partial_cmp(b),
            (DeadlineHint::Some(_), DeadlineHint::Background) => Some(std::cmp::Ordering::Less),
            (DeadlineHint::Background, DeadlineHint::Some(_)) => Some(std::cmp::Ordering::Greater),
            (DeadlineHint::Background, DeadlineHint::Background) => Some(std::cmp::Ordering::Equal),
        }
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
        assert_eq!(DeadlineHint::Some(1), DeadlineHint::Some(1));
        assert_ne!(DeadlineHint::Some(1), DeadlineHint::Some(2));
        assert_ne!(DeadlineHint::Some(1), DeadlineHint::Background);
        assert_ne!(DeadlineHint::Background, DeadlineHint::Some(1));
        assert_eq!(DeadlineHint::Background, DeadlineHint::Background);
    }

    #[test]
    fn test_deadline_hint_partial_ord() {
        assert_eq!(
            DeadlineHint::Some(1).partial_cmp(&DeadlineHint::Some(1)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            DeadlineHint::Some(1).partial_cmp(&DeadlineHint::Some(2)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            DeadlineHint::Some(2).partial_cmp(&DeadlineHint::Some(1)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            DeadlineHint::Some(1).partial_cmp(&DeadlineHint::Background),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            DeadlineHint::Background.partial_cmp(&DeadlineHint::Some(1)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            DeadlineHint::Background.partial_cmp(&DeadlineHint::Background),
            Some(std::cmp::Ordering::Equal)
        );
    }
}
