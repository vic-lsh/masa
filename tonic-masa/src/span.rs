use crate::{Distribution, Latency, LatencyTracker, SpanId, ONLINE_TRACKER};

/// Represent a span inner.
#[derive(Debug, Default, Clone)]
pub struct Span {
    span_id: SpanId,
    distribution: Option<Distribution>,
    tracker_capacity: Option<usize>,
}

impl Span {
    /// Create a new span inner.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        tracker_capacity: Option<usize>,
    ) -> Self {
        Self {
            span_id,
            distribution,
            tracker_capacity,
        }
    }

    /// Get the span ID.
    pub fn span_id(&self) -> &SpanId {
        &self.span_id
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
    span_id: SpanId,
    distribution: Option<Distribution>,
    tracker: Option<LatencyTracker>,
}

impl From<Span> for SpanTracker {
    fn from(span: Span) -> Self {
        SpanTracker::new(span.span_id, span.distribution, span.tracker_capacity)
    }
}

impl SpanTracker {
    /// Create a new span.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        tracker_capacity: Option<usize>,
    ) -> Self {
        let mut tracker = None;
        if ONLINE_TRACKER {
            let tracker_capacity = tracker_capacity.unwrap();
            tracker = Some(LatencyTracker::new(tracker_capacity));
        }
        Self {
            span_id,
            distribution,
            tracker,
        }
    }

    /// Get the span ID.
    pub fn span_id(&self) -> &SpanId {
        &self.span_id
    }

    /// Get the distribution.
    pub fn distribution(&self) -> &Distribution {
        assert!(self.distribution.is_some());
        self.distribution.as_ref().unwrap()
    }

    /// Estimate the latency.
    pub fn estimate(&self) -> Latency {
        if ONLINE_TRACKER {
            self.tracker.as_ref().unwrap().estimate()
        } else {
            self.distribution().mean()
        }
    }

    /// Update the tracker.
    pub fn track(&mut self, latency: Latency) {
        if ONLINE_TRACKER {
            self.tracker.as_mut().unwrap().track(latency);
        } else {
            panic!("Not implemented");
        }
    }
}
