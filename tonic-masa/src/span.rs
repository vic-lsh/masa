use crate::{Distribution, Latency, LatencyTracker, SpanId};

/// Represent a span inner.
#[derive(Debug, Default, Clone)]
pub struct Span {
    span_id: SpanId,
    distribution: Option<Distribution>,
    capacity: usize,
    pctl_future: usize,
    pctl_present: usize,
}

impl Span {
    /// Create a new span inner.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        capacity: usize,
        pctl_future: usize,
        pctl_present: usize,
    ) -> Self {
        Self {
            span_id,
            distribution,
            capacity,
            pctl_future,
            pctl_present,
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
    pctl_future: usize,
    pctl_present: usize,
}

impl From<Span> for SpanTracker {
    fn from(span: Span) -> Self {
        SpanTracker::new(
            span.span_id,
            span.distribution,
            span.capacity,
            span.pctl_future,
            span.pctl_present,
        )
    }
}

impl SpanTracker {
    /// Create a new span.
    pub fn new(
        span_id: SpanId,
        distribution: Option<Distribution>,
        capacity: usize,
        pctl_future: usize,
        pctl_present: usize,
    ) -> Self {
        let tracker = LatencyTracker::new(span_id.clone(), capacity);
        Self {
            span_id,
            distribution,
            tracker,
            pctl_future,
            pctl_present,
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

    /// Estimate the latency for the deadline.
    #[inline]
    pub fn estimate_deadline(&self) -> Latency {
        self.tracker.estimate(self.pctl_future)
    }

    /// Estimate the latency for the latest exec.
    #[inline]
    pub fn estimate_latest_exec(&self) -> Latency {
        self.tracker.estimate(self.pctl_present)
    }

    /// Update the tracker.
    #[inline]
    pub fn track(&mut self, latency: Latency) {
        self.tracker.track(latency);
    }
}

/// Represent a span.
#[derive(Debug, Default)]
pub struct FutureSpanTracker {
    span_id: SpanId,
    tracker_future: LatencyTracker,
    tracker_present: LatencyTracker,
    pctl_future: usize,
    pctl_present: usize,
}

impl From<Span> for FutureSpanTracker {
    fn from(span: Span) -> Self {
        FutureSpanTracker::new(
            span.span_id,
            span.capacity,
            span.pctl_future,
            span.pctl_present,
        )
    }
}

impl FutureSpanTracker {
    /// Create a new span.
    pub fn new(span_id: SpanId, capacity: usize, pctl_future: usize, pctl_present: usize) -> Self {
        let tracker_future = LatencyTracker::new(span_id.clone(), capacity);
        let tracker_present = LatencyTracker::new(span_id.clone(), capacity);
        Self {
            span_id,
            tracker_future,
            tracker_present,
            pctl_future,
            pctl_present,
        }
    }

    /// Get the span ID.
    pub fn span_id(&self) -> &SpanId {
        &self.span_id
    }

    /// Estimate the latency for the deadline.
    #[inline]
    pub fn estimate_future(&self) -> Latency {
        self.tracker_future.estimate(self.pctl_future)
    }

    /// Estimate the latency for the latest exec.
    #[inline]
    pub fn estimate_present(&self) -> Latency {
        self.tracker_present.estimate(self.pctl_present)
    }

    /// Track the future.
    #[inline]
    pub fn track_future(&mut self, latency: Latency) {
        self.tracker_future.track(latency);
    }

    /// Track the present.
    #[inline]
    pub fn track_present(&mut self, latency: Latency) {
        self.tracker_present.track(latency);
    }
}
