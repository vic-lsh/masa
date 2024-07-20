use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

/// Type alias for a path.
pub type Path = String;

/// Type alias for a timestamp.
pub type Timestamp = u64;

/// Type alias for a latency.
pub type Latency = u64;

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Span {
    path: Path,
    proc_est: Latency,
    proc_elapse: Latency,
}

impl Span {
    /// Create a new span.
    pub fn new(path: Path, proc_est: Latency, proc_elapse: Latency) -> Self {
        Self {
            path,
            proc_est,
            proc_elapse,
        }
    }
}

/// Represent a call graph in a local view.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LocalGraph {
    spans: Vec<Span>,
    paths: Vec<Path>,
}

impl LocalGraph {
    /// Create a new graph.
    pub fn new(spans: Vec<Span>) -> Self {
        let paths = spans.iter().map(|x| x.path.clone()).collect();
        Self { spans, paths }
    }

    /// Return the estimated suffix latency after a span indexed by its path.
    pub fn estimate_suffix(&mut self, path: &Path) -> Latency {
        assert!(self.paths.contains(path));
        let id = self.paths.iter().position(|x| x == path).unwrap();

        let mut suffix_sum = 0;
        for i in id + 1..self.paths.len() {
            suffix_sum += self.spans[i].proc_est;
        }

        suffix_sum
    }
}

/// Represent a call graph in a global view.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GlobalGraph {
    local_graphs: HashMap<Path, LocalGraph>,
}

impl GlobalGraph {
    /// Create a new graph.
    pub fn new(local_graphs: HashMap<Path, LocalGraph>) -> Self {
        assert!(local_graphs.contains_key("Source"));
        Self { local_graphs }
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
    start_at: Timestamp,
    deadline: Timestamp,
    local_graph: LocalGraph,
    global_graph: GlobalGraph,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        start_at: Timestamp,
        deadline: Timestamp,
        local_graph: LocalGraph,
        global_graph: GlobalGraph,
    ) -> Self {
        Self {
            start_at,
            deadline,
            local_graph,
            global_graph,
        }
    }

    /// Create a new Masa context into a spawned span indexed by its path.
    pub fn spawn(&mut self, path: &Path) -> Self {
        let deadline = self.deadline - self.local_graph.estimate_suffix(path);
        Context::new(
            self.start_at,
            deadline,
            self.global_graph.get_local_graph(path).clone(),
            self.global_graph.clone(),
        )
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
