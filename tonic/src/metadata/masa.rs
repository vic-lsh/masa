use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

type Timestamp = u64;
type Latency = u64;
type Span = String;

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MasaContext {
    span: Span,
    start_at: Timestamp,
    deadline: Timestamp,
    graph: Graph,
}

impl MasaContext {
    /// Create a default Masa context.
    pub fn default() -> Self {
        Self {
            span: "0".to_string(),
            start_at: 1,
            deadline: 2,
            graph: Graph::default(),
        }
    }

    /// Create a new Masa context.
    pub fn new(span: Span, start_at: Timestamp, deadline: Timestamp, graph: Graph) -> Self {
        Self {
            span,
            start_at,
            deadline,
            graph,
        }
    }

    /// Create a new Masa context with a forward span.
    pub fn forward(&self, span: Span) -> Self {
        let ctx = MasaContext {
            span,
            start_at: self.start_at + 1,
            deadline: self.deadline + 2,
            graph: self.graph.clone(),
        };
        ctx

        // [TODO] Check self.span and span are valid.
        // let deadline = self.deadline - graph.proc_ests[&span];
        // let ctx = Masa context {
        // 	span,
        // 	start_at: self.start_at,
        // 	deadline,
        // 	graph: self.graph,
        // };
        // ctx
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
}

impl Graph {
    /// Create a default graph.
    pub fn default() -> Self {
        Self {
            spans: Vec::new(),
            proc_ests: HashMap::new(),
            proc_elapses: HashMap::new(),
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
        }
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
