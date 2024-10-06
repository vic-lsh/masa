use std::collections::HashMap;

use crate::{GraphId, Latency, MethodId, Span, SpanTracker};

/// Represent a local graph inner.
#[derive(Debug, Default, Clone)]
pub struct LocalGraph {
    graph_id: GraphId,
    method_id: MethodId,
    spans: Vec<Span>,
}

impl LocalGraph {
    /// Create a new local graph inner.
    pub fn new(graph_id: GraphId, method_id: MethodId, spans: Vec<Span>) -> Self {
        Self {
            graph_id,
            method_id,
            spans,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphId {
        &self.graph_id
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
    graph_id: GraphId,
    method_id: MethodId,
    spans: Vec<SpanTracker>,
}

impl From<LocalGraph> for LocalGraphTracker {
    fn from(local_graph: LocalGraph) -> Self {
        let graph_id = local_graph.graph_id().clone();
        let method_id = local_graph.method_id().clone();
        let spans = local_graph
            .spans
            .iter()
            .map(|span| SpanTracker::from(span.clone()))
            .collect();
        Self::new(graph_id, method_id, spans)
    }
}

impl LocalGraphTracker {
    /// Create a new graph.
    pub fn new(graph_id: GraphId, method_id: MethodId, spans: Vec<SpanTracker>) -> Self {
        Self {
            graph_id,
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
            "estimate_suffix_deadline, graph_id: {:?}, method_id: {:?}, child_method_id: {:?}, suffix_sum: {}",
            self.graph_id,
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
            "estimate_suffix_latest_exec_at, graph_id: {:?}, method_id: {:?}, child_method_id: {:?}, suffix_sum: {}",
            self.graph_id,
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
            "tracker, graph_id: {:?}, method_id: {:?}, child_method_id: {:?}, latency: {} us",
            self.graph_id,
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
    graph_id: GraphId,
    local_graphs: HashMap<MethodId, LocalGraph>,
    slo: Latency,
}

impl GlobalGraph {
    /// Create a new global graph inner.
    pub fn new(graph_id: GraphId, local_graphs: HashMap<MethodId, LocalGraph>) -> Self {
        Self {
            graph_id,
            local_graphs,
            slo: 0,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphId {
        &self.graph_id
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
    graph_id: GraphId,
    local_graphs: HashMap<MethodId, LocalGraphTracker>,
}

impl From<GlobalGraph> for GlobalGraphTracker {
    fn from(global_graph: GlobalGraph) -> Self {
        let local_graphs = global_graph
            .local_graphs
            .iter()
            .map(|(path, local_graph)| (path.clone(), LocalGraphTracker::from(local_graph.clone())))
            .collect();
        Self::new(global_graph.graph_id, local_graphs)
    }
}

impl GlobalGraphTracker {
    /// Create a new graph.
    pub fn new(graph_id: GraphId, local_graphs: HashMap<MethodId, LocalGraphTracker>) -> Self {
        assert!(local_graphs.contains_key(&"Source".to_string()));
        Self {
            graph_id,
            local_graphs,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphId {
        &self.graph_id
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: &MethodId) -> &LocalGraphTracker {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }
}
