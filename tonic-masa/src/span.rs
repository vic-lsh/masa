use std::sync::RwLock;

use crate::{Distribution, Latency, LatencyTracker, Path, EST_OFFLINE, EST_ONLINE, MOCK_DIST};

/// Represent a span inner.
#[derive(Debug, Default, Clone)]
pub struct SpanInner {
    path: Path,
    distribution: Option<Distribution>,
    tracker_capacity: Option<usize>,
}

impl SpanInner {
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
}

/// Represent a span.
#[derive(Debug, Default)]
pub struct Span {
    path: Path,
    distribution: Option<Distribution>,
    tracker: Option<RwLock<LatencyTracker>>,
}

impl From<SpanInner> for Span {
    fn from(span: SpanInner) -> Self {
        Span::new(span.path, span.distribution, span.tracker_capacity)
    }
}

impl Span {
    /// Create a new span.
    pub fn new(
        path: Path,
        distribution: Option<Distribution>,
        tracker_capacity: Option<usize>,
    ) -> Self {
        if MOCK_DIST {
            assert!(distribution.is_some());
        }
        let mut tracker = None;
        if EST_ONLINE {
            let tracker_capacity = tracker_capacity.unwrap();
            tracker = Some(RwLock::new(LatencyTracker::new(tracker_capacity)));
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
    pub fn get_distribution(&self) -> &Distribution {
        assert!(self.distribution.is_some());
        self.distribution.as_ref().unwrap()
    }

    /// Estimate the latency.
    pub fn estimate(&self) -> Latency {
        if EST_OFFLINE {
            self.get_distribution().estimate()
        } else if EST_ONLINE {
            self.tracker.as_ref().unwrap().read().unwrap().estimate()
        } else {
            panic!("Not implemented")
        }
    }

    /// Update the tracker.
    pub fn track(&mut self, latency: Latency) {
        self.tracker.as_ref().unwrap().write().unwrap().add(latency);
    }
}
