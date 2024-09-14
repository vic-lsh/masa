use crate::{Distribution, Latency, LatencyTracker, Path, EST_ONLINE, MOCK_DIST};

/// Represent a span inner.
#[derive(Debug, Default, Clone)]
pub struct Span {
    path: Path,
    distribution: Option<Distribution>,
    tracker_capacity: Option<usize>,
}

impl Span {
    /// Create a new span inner.
    pub fn new(
        path: Path,
        distribution: Option<Distribution>,
        tracker_capacity: Option<usize>,
    ) -> Self {
        Self {
            path,
            distribution,
            tracker_capacity,
        }
    }

    /// Get the path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the distribution.
    pub fn distribution(&self) -> &Distribution {
        assert!(self.distribution.is_some());
        self.distribution.as_ref().unwrap()
    }
}

/// Represent a span.
#[derive(Debug, Default)]
pub struct SpanTracker {
    path: Path,
    distribution: Option<Distribution>,
    tracker: Option<LatencyTracker>,
}

impl From<Span> for SpanTracker {
    fn from(span: Span) -> Self {
        SpanTracker::new(span.path, span.distribution, span.tracker_capacity)
    }
}

impl SpanTracker {
    /// Create a new span.
    pub fn new(
        path: Path,
        distribution: Option<Distribution>,
        tracker_capacity: Option<usize>,
    ) -> Self {
        let mut tracker = None;
        if EST_ONLINE {
            let tracker_capacity = tracker_capacity.unwrap();
            tracker = Some(LatencyTracker::new(tracker_capacity));
        }
        Self {
            path,
            distribution,
            tracker,
        }
    }

    /// Get the path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the distribution.
    pub fn distribution(&self) -> &Distribution {
        assert!(self.distribution.is_some());
        self.distribution.as_ref().unwrap()
    }

    /// Estimate the latency.
    pub fn estimate(&self) -> Latency {
        if EST_ONLINE {
            self.tracker.as_ref().unwrap().estimate()
        } else if MOCK_DIST {
            self.distribution().mean()
        } else {
            panic!("Not implemented");
        }
    }

    /// Update the tracker.
    pub fn track(&mut self, latency: Latency) {
        if EST_ONLINE {
            self.tracker.as_mut().unwrap().track(latency);
        } else {
            panic!("Not implemented");
        }
    }
}
