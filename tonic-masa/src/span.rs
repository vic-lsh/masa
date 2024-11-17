use crate::{Distribution, Latency, LatencyTracker, SpanId};

/// Represent a span inner.
#[derive(Debug, Default, Clone)]
pub struct Span {
    span_id: SpanId,
    distribution: Option<Distribution>,
    capacity: usize,
    pctl_deadline: usize,
    pctl_latest_exec: usize,
}

impl Span {
    /// Create a new span inner.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        capacity: usize,
        pctl_deadline: usize,
        pctl_latest_exec: usize,
    ) -> Self {
        Self {
            span_id,
            distribution,
            capacity,
            pctl_deadline,
            pctl_latest_exec,
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
    tracker: LatencyTracker,
    pctl_deadline: usize,
    pctl_latest_exec: usize,
}

impl From<Span> for SpanTracker {
    fn from(span: Span) -> Self {
        SpanTracker::new(
            span.span_id,
            span.distribution,
            span.capacity,
            span.pctl_deadline,
            span.pctl_latest_exec,
        )
    }
}

impl SpanTracker {
    /// Create a new span.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        capacity: usize,
        pctl_deadline: usize,
        pctl_latest_exec: usize,
    ) -> Self {
        let tracker = LatencyTracker::new(span_id.clone(), capacity);
        Self {
            span_id,
            distribution,
            tracker,
            pctl_deadline,
            pctl_latest_exec,
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
    #[inline]
    pub fn estimate(&self) -> Latency {
        panic!("Deprecated");
        // self.distribution().mean()
    }

    /// Estimate the latency for the deadline.
    #[inline]
    pub fn estimate_deadline(&self) -> Latency {
        self.tracker.estimate(self.pctl_deadline)
    }

    /// Estimate the latency for the latest exec.
    #[inline]
    pub fn estimate_latest_exec(&self) -> Latency {
        self.tracker.estimate(self.pctl_latest_exec)
    }

    /// Update the tracker.
    #[inline]
    pub fn track(&mut self, latency: Latency) {
        self.tracker.track(latency);
    }
}
