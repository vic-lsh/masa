use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

/// Type alias for a path.
pub type Path = String;

/// Type alias for a timestamp.
pub type Timestamp = u64;

/// Type alias for a latency.
pub type Latency = u64;

/// Type alias for a graph ID.
pub type GraphID = String;

/// Type alias for an addreess.
pub type Address = String;

/// Type alias for a request ID.
pub type RequestID = u64;

/// Represent a distribution.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Distribution {
    mean: Latency,
    percentile_latencies: Option<Vec<Latency>>,
}

impl Distribution {
    /// Create a new distribution.
    pub fn new(mean: Latency, percentile_latencies: Option<Vec<Latency>>) -> Self {
        Self {
            mean,
            percentile_latencies,
        }
    }

    /// Get an estimate.
    pub fn estimate(&self) -> Latency {
        self.mean
    }

    /// Sample a latency.
    pub fn sample(&self, request_id: RequestID) -> Latency {
        if let Some(percentile_latencies) = &self.percentile_latencies {
            let index = request_id as usize % percentile_latencies.len();
            percentile_latencies[index]
        } else {
            panic!("No percentile latencies");
        }
    }
}

/// Represent a span.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Span {
    path: Path,
    distribution: Distribution,
}

impl Span {
    /// Create a new span.
    pub fn new(path: Path, distribution: Distribution) -> Self {
        Self { path, distribution }
    }

    /// Get the path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the distribution.
    pub fn distribution(&self) -> &Distribution {
        &self.distribution
    }
}

/// Represent a call graph in a local view.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LocalGraph {
    spans: Vec<Span>,
}

impl LocalGraph {
    /// Create a new graph.
    pub fn new(spans: Vec<Span>) -> Self {
        Self { spans }
    }

    /// Return the estimated suffix latency after a span indexed by its path.
    pub fn estimate_suffix(&self, path: &Path) -> Latency {
        let mut existed = false;
        let mut suffix_sum = 0;

        for span in self.spans.iter().rev() {
            if span.path == *path {
                existed = true;
                break;
            }
            suffix_sum += span.distribution().estimate();
        }

        assert!(existed);
        suffix_sum
    }

    /// Get the spans.
    pub fn spans(&self) -> &Vec<Span> {
        &self.spans
    }
}

/// Represent a call graph in a global view.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GlobalGraph {
    graph_id: GraphID,
    local_graphs: HashMap<Path, LocalGraph>,
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

    /// Get the source local graph.
    pub fn get_source(&self) -> &LocalGraph {
        self.get_local_graph(&"Source".to_string())
    }

    /// Get a local graph indexed by its path.
    pub fn get_local_graph(&self, path: &Path) -> &LocalGraph {
        assert!(self.local_graphs.contains_key(path));
        &self.local_graphs[path]
    }
}

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    graph_id: GraphID,
    request_id: RequestID,
    start_at: Timestamp,
    deadline: Timestamp,
    local_graph: Option<LocalGraph>,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        graph_id: GraphID,
        request_id: RequestID,
        start_at: Timestamp,
        deadline: Timestamp,
        local_graph: Option<LocalGraph>,
    ) -> Self {
        Self {
            graph_id,
            request_id,
            start_at,
            deadline,
            local_graph,
        }
    }

    /// Get the graph ID.
    pub fn graph_id(&self) -> &GraphID {
        &self.graph_id
    }

    /// Get the request ID.
    pub fn request_id(&self) -> RequestID {
        self.request_id
    }

    /// Get the start timestamp.
    pub fn start_at(&self) -> Timestamp {
        self.start_at
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    /// Get the local graph.
    pub fn get_local_graph(&self) -> LocalGraph {
        self.local_graph.clone().unwrap()
    }

    /// Set the local graph.
    pub fn set_local_graph(&mut self, local_graph: LocalGraph) {
        self.local_graph = Some(local_graph);
    }

    /// Create a new Masa context from JSON.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    /// Convert a Masa context to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }
}
