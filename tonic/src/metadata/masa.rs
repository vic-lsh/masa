use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

/// Type alias for a timestamp.
pub type Timestamp = u64;

/// Type alias for a latency.
pub type Latency = u64;

/// Type alias for a span.
pub type Span = String;

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    start_at: Timestamp,
    deadline: Timestamp,
    graph: Graph,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(start_at: Timestamp, deadline: Timestamp, graph: Graph) -> Self {
        Self {
            start_at,
            deadline,
            graph: graph,
        }
    }

    /// Create a new Masa context with a forward span.
    pub fn forward(&mut self, span: &Span) -> Self {
        let deadline = self.graph.forward(span, self.deadline);
        Context::new(self.start_at, deadline, Graph::default())
    }

    /// Set the graph of a Masa context.
    pub fn set_graph(&mut self, graph: Graph) {
        self.graph = graph;
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

/// Represent a call graph.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Graph {
    spans: Vec<Span>,
    proc_ests: HashMap<Span, Latency>,
    proc_elapses: HashMap<Span, Latency>,
    id: usize,
}

impl Graph {
    /// Create a default graph.
    pub fn default() -> Self {
        let spans = vec!["head".to_string(), "tail".to_string()];
        let mut proc_ests = HashMap::new();
        for span in &spans {
            proc_ests.insert(span.clone(), 1);
        }
        let mut proc_elapses = HashMap::new();
        for span in &spans {
            proc_elapses.insert(span.clone(), 1);
        }
        Self {
            spans,
            proc_ests,
            proc_elapses,
            id: 0,
        }
    }

    /// Create a new graph.
    pub fn new(
        spans: Vec<Span>,
        proc_ests: HashMap<Span, Latency>,
        proc_elapses: HashMap<Span, Latency>,
    ) -> Self {
        Self {
            spans,
            proc_ests,
            proc_elapses,
            id: 0,
        }
    }

    /// Return the deadline of a forward span.
    pub fn forward(&mut self, span: &Span, deadline: Timestamp) -> Timestamp {
        assert!(self.spans.contains(span));
        let id = self.spans.iter().position(|x| x == span).unwrap();
        assert!(self.id + 1 == id);

        let mut suffix_sum = 0;
        for i in id + 1..self.spans.len() {
            suffix_sum += self.proc_ests[&self.spans[i]];
        }

        self.id += 1;

        let deadline = deadline - suffix_sum;
        deadline
    }

    /// Create a new graph from JSON.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    /// Convert a graph to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }
}
