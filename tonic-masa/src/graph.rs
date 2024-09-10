use std::collections::HashMap;

use crate::{GraphID, Latency, Path, Span, SpanInner};

/// Represent a local graph inner.
#[derive(Debug, Default, Clone)]
pub struct LocalGraphInner {
    spans: Vec<SpanInner>,
}

impl LocalGraphInner {
    /// Create a new local graph inner.
    pub fn new(spans: Vec<SpanInner>) -> Self {
        Self { spans }
    }
}

/// Represent a local graph.
#[derive(Debug, Default)]
pub struct LocalGraph {
    spans: Vec<Span>,
}

impl From<LocalGraphInner> for LocalGraph {
    fn from(local_graph: LocalGraphInner) -> Self {
        let spans = local_graph
            .spans
            .iter()
            .map(|span| Span::from(span.clone()))
            .collect();
        Self::new(spans)
    }
}

impl LocalGraph {
    /// Create a new graph.
    pub fn new(spans: Vec<Span>) -> Self {
        Self { spans }
    }

    /// Get the spans.
    pub fn spans(&self) -> &Vec<Span> {
        &self.spans
    }

    /// Return the estimated suffix latency after a span indexed by its path.
    pub fn estimate_suffix(&self, path: &Path) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;

        for span in self.spans.iter().rev() {
            if *span.path() == *path {
                existed = true;
                break;
            }
            suffix_sum += span.estimate();
        }

        assert!(existed, "Span {} not found", path);
        suffix_sum
    }

    /// Track the latency of a span indexed by its path.
    pub fn track(&mut self, path: &Path, latency: Latency) {
        for span in self.spans.iter_mut() {
            if *span.path() == *path {
                span.track(latency);
                return;
            }
        }
        panic!("Span {} not found", path);
    }
}

/// Represent a global graph inner.
#[derive(Debug, Default, Clone)]
pub struct GlobalGraphInner {
    graph_id: GraphID,
    local_graphs: HashMap<Path, LocalGraphInner>,
}

impl GlobalGraphInner {
    /// Create a new global graph inner.
    pub fn new(graph_id: GraphID, local_graphs: HashMap<Path, LocalGraphInner>) -> Self {
        Self {
            graph_id,
            local_graphs,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphID {
        &self.graph_id
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: &Path) -> &LocalGraphInner {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }

    /// Get the source local graph.
    pub fn get_source(&self) -> &LocalGraphInner {
        self.get_local_graph(&"Source".to_string())
    }
}

/// Represent a global graph.
#[derive(Debug, Default)]
pub struct GlobalGraph {
    graph_id: GraphID,
    local_graphs: HashMap<Path, LocalGraph>,
}

impl From<GlobalGraphInner> for GlobalGraph {
    fn from(global_graph: GlobalGraphInner) -> Self {
        let local_graphs = global_graph
            .local_graphs
            .iter()
            .map(|(path, local_graph)| (path.clone(), LocalGraph::from(local_graph.clone())))
            .collect();
        Self::new(global_graph.graph_id, local_graphs)
    }
}

impl GlobalGraph {
    /// Create a new graph.
    pub fn new(graph_id: GraphID, local_graphs: HashMap<Path, LocalGraph>) -> Self {
        assert!(local_graphs.contains_key(&"Source".to_string()));
        Self {
            graph_id,
            local_graphs,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphID {
        &self.graph_id
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: &Path) -> &LocalGraph {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }

    /// Get the source local graph.
    pub fn get_source(&self) -> &LocalGraph {
        self.get_local_graph(&"Source".to_string())
    }
}
