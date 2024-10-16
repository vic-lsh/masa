use std::collections::HashMap;

use crate::{Latency, MethodId, ServiceId, Span, SpanTracker};

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
    pub fn method_id(&self) -> &MethodId {
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
    fn from(local_graph: LocalGraph) -> Self {
        let service_id = local_graph.service_id().clone();
        let method_id = local_graph.method_id().clone();
        let spans = local_graph
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
    pub fn estimate_suffix_deadline(&self, child_method_id: &MethodId) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;
        for span in self.spans.iter().rev() {
            if span.span_id() == child_method_id {
                existed = true;
                break;
            }
            suffix_sum += span.estimate();
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
    pub fn estimate_suffix_latest_exec_at(&self, child_method_id: &MethodId) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;
        for span in self.spans.iter().rev() {
            suffix_sum += span.estimate();
            if span.span_id() == child_method_id {
                existed = true;
                break;
            }
        }
        log::info!(
            "estimate_suffix_latest_exec_at, service_id: {:?}, method_id: {:?}, child_method_id: {:?}, suffix_sum: {}",
            self.service_id,
            self.method_id,
            child_method_id,
            suffix_sum
        );
        assert!(existed, "Span {} not found", child_method_id);
        suffix_sum
    }

    /// Track the latency of a span indexed by its path.
    pub fn track_span(&mut self, child_method_id: &MethodId, latency_us: Latency) {
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
    pub fn contains_path(&self, path: &MethodId) -> bool {
        self.local_graphs.contains_key(path)
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: &MethodId) -> &LocalGraph {
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
            .map(|(path, local_graph)| (path.clone(), LocalGraphTracker::from(local_graph.clone())))
            .collect();
        Self::new(global_graph.service_id, local_graphs)
    }
}

impl GlobalGraphTracker {
    /// Create a new graph.
    pub fn new(service_id: ServiceId, local_graphs: HashMap<MethodId, LocalGraphTracker>) -> Self {
        assert!(local_graphs.contains_key(&"Source".to_string()));
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
    pub fn get_local_graph(&self, path: &MethodId) -> &LocalGraphTracker {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }
}
