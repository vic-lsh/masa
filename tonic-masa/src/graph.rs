use std::collections::HashMap;

use crate::{FutureSpanTracker, Latency, MethodId, ServiceId, Span, SpanId, SpanTracker};

/// Represent a local graph inner.
#[derive(Debug, Default, Clone)]
pub struct LocalGraph {
    service_id: ServiceId,
    method_id: MethodId,
    spans: Vec<Span>,
}

impl LocalGraph {
    /// Create a new local graph inner.
    pub fn new(service_id: ServiceId, method_id: MethodId, spans: Vec<Span>) -> Self {
        Self {
            service_id,
            method_id,
            spans,
        }
    }

    /// Get the service ID.
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    /// Get the method ID.
    pub fn method_id(&self) -> MethodId {
        &self.method_id
    }

    /// Get the spans.
    pub fn spans(&self) -> &Vec<Span> {
        &self.spans
    }
}

/// Represent a local graph.
#[derive(Debug, Default)]
pub struct LocalGraphTracker {
    service_id: ServiceId,
    method_id: MethodId,
    spans: Vec<SpanTracker>,
}

impl From<LocalGraph> for LocalGraphTracker {
    fn from(graph: LocalGraph) -> Self {
        let service_id = graph.service_id().clone();
        let method_id = graph.method_id();
        let spans = graph
            .spans
            .iter()
            .map(|span| SpanTracker::from(span.clone()))
            .collect();
        Self::new(service_id, method_id, spans)
    }
}

impl LocalGraphTracker {
    /// Create a new graph.
    pub fn new(service_id: ServiceId, method_id: MethodId, spans: Vec<SpanTracker>) -> Self {
        Self {
            service_id,
            method_id,
            spans,
        }
    }

    /// Get the spans.
    pub fn spans(&self) -> &Vec<SpanTracker> {
        &self.spans
    }

    /// Return the estimated suffix latency after a span indexed by its path.
    pub fn estimate_suffix_deadline(&self, child_method_id: MethodId) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;
        for span in self.spans.iter().rev() {
            if span.span_id() == child_method_id {
                existed = true;
                break;
            }
            suffix_sum += span.estimate_deadline();
        }
        log::info!(
            "estimate_suffix_deadline, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, suffix_sum: {}",
            self.service_id,
            self.method_id,
            child_method_id,
            suffix_sum
        );
        assert!(existed, "Span {} not found", child_method_id);
        suffix_sum
    }

    /// Return the estimated suffix latency no before than a span indexed by its path.
    pub fn estimate_suffix_latest_exec(&self, child_method_id: MethodId) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;
        for span in self.spans.iter().rev() {
            suffix_sum += span.estimate_latest_exec();
            if span.span_id() == child_method_id {
                existed = true;
                break;
            }
        }
        log::info!(
            "estimate_suffix_latest_exec, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, suffix_sum: {}",
            self.service_id,
            self.method_id,
            child_method_id,
            suffix_sum
        );
        assert!(existed, "Span {} not found", child_method_id);
        suffix_sum
    }

    /// Track the latency of a span indexed by its path.
    pub fn track_span(&mut self, child_method_id: MethodId, latency_us: Latency) {
        log::info!(
            "tracker, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, latency: {} us",
            self.service_id,
            self.method_id,
            child_method_id,
            latency_us
        );
        for span in self.spans.iter_mut() {
            if span.span_id() == child_method_id {
                span.track(latency_us);
                return;
            }
        }
        panic!("Span {} not found", child_method_id);
    }
}

/// Represent a global graph inner.
#[derive(Debug, Default, Clone)]
pub struct GlobalGraph {
    service_id: ServiceId,
    local_graphs: HashMap<MethodId, LocalGraph>,
    slo: Latency,
}

impl GlobalGraph {
    /// Create a new global graph inner.
    pub fn new(service_id: ServiceId, local_graphs: HashMap<MethodId, LocalGraph>) -> Self {
        Self {
            service_id,
            local_graphs,
            slo: 0,
        }
    }

    /// Get the service ID.
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    /// Get the local graphs.
    pub fn local_graphs(&self) -> &HashMap<MethodId, LocalGraph> {
        &self.local_graphs
    }

    /// Check if a path is contained in the graph.
    pub fn contains_path(&self, path: MethodId) -> bool {
        self.local_graphs.contains_key(path)
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: MethodId) -> &LocalGraph {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }

    /// Set the SLO.
    pub fn set_slo(&mut self, slo: Latency) {
        self.slo = slo;
    }

    /// Get the SLO.
    pub fn slo(&self) -> Latency {
        self.slo
    }
}

/// Represent a global graph.
#[derive(Debug, Default)]
pub struct GlobalGraphTracker {
    service_id: ServiceId,
    local_graphs: HashMap<MethodId, LocalGraphTracker>,
}

impl From<GlobalGraph> for GlobalGraphTracker {
    fn from(global_graph: GlobalGraph) -> Self {
        let local_graphs = global_graph
            .local_graphs
            .iter()
            .map(|(path, local_graph)| (*path, LocalGraphTracker::from(local_graph.clone())))
            .collect();
        Self::new(global_graph.service_id, local_graphs)
    }
}

impl GlobalGraphTracker {
    /// Create a new graph.
    pub fn new(service_id: ServiceId, local_graphs: HashMap<MethodId, LocalGraphTracker>) -> Self {
        assert!(local_graphs.contains_key("Source"));
        Self {
            service_id,
            local_graphs,
        }
    }

    /// Get the service ID.
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: MethodId) -> &LocalGraphTracker {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }
}

/// Represent a future graph tracker.
#[derive(Debug, Default)]
pub struct FutureGraphTracker {
    service_id: ServiceId,
    method_id: MethodId,
    spans: HashMap<SpanId, FutureSpanTracker>,
}

impl From<LocalGraph> for FutureGraphTracker {
    fn from(graph: LocalGraph) -> Self {
        let service_id = graph.service_id().clone();
        let method_id = graph.method_id();
        let spans = graph
            .spans
            .iter()
            .map(|span| {
                (
                    span.span_id().clone(),
                    FutureSpanTracker::from(span.clone()),
                )
            })
            .collect();
        Self::new(service_id, method_id, spans)
    }
}

impl FutureGraphTracker {
    /// Create a new graph.
    pub fn new(
        service_id: ServiceId,
        method_id: MethodId,
        spans: HashMap<SpanId, FutureSpanTracker>,
    ) -> Self {
        Self {
            service_id,
            method_id,
            spans,
        }
    }

    /// Estimate the future latency after a span.
    pub fn estimate_future(&self, child_method_id: MethodId) -> Latency {
        let span = self.spans.get(child_method_id).unwrap();
        let latency = span.estimate_future();
        log::info!(
            "estimate_future, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, latency: {} us",
            self.service_id,
            self.method_id,
            child_method_id,
            latency
        );
        return latency;
    }

    /// Estimate the present latency of a span.
    pub fn estimate_present(&self, child_method_id: MethodId) -> Latency {
        let span = self.spans.get(child_method_id).unwrap();
        let latency = span.estimate_present();
        log::info!(
            "estimate_present, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, latency: {} us",
            self.service_id,
            self.method_id,
            child_method_id,
            latency
        );
        return latency;
    }

    /// Track the future latency after a span.
    pub fn track_future_span(&mut self, child_method_id: MethodId, latency_us: Latency) {
        let span = self.spans.get_mut(child_method_id).unwrap();
        span.track_future(latency_us);
    }

    /// Track the present latency of a span.
    pub fn track_present_span(&mut self, child_method_id: MethodId, latency_us: Latency) {
        let span = self.spans.get_mut(child_method_id).unwrap();
        span.track_present(latency_us);
    }
}
