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
            span: "head".to_string(),
            start_at: 1,
            deadline: 10,
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
    pub fn forward(&mut self, span: &Span) -> Self {
        let graph = &self.graph;
        assert!(graph.spans.contains(span));

        let prev_id = graph.spans.iter().position(|x| x == &self.span).unwrap();
        let next_id = graph.spans.iter().position(|x| x == span).unwrap();
        assert!(prev_id + 1 == next_id);

        let mut suffix_sum = 0;
        for i in next_id + 1..graph.spans.len() {
            suffix_sum += graph.proc_ests[&graph.spans[i]];
        }
        let deadline = self.deadline - suffix_sum;

        self.span = span.clone();
        MasaContext {
            span: span.clone(),
            start_at: self.start_at,
            deadline,
            graph: self.graph.clone(),
        }
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
        let spans = vec![
            "head".to_string(),
            "/hello.Greeter/SayHello".to_string(),
            "tail".to_string(),
        ];
        let mut proc_ests = HashMap::new();
        for i in 0..spans.len() {
            proc_ests.insert(spans[i].clone(), i as Latency);
        }
        let mut proc_elapses = HashMap::new();
        for i in 0..spans.len() {
            proc_elapses.insert(spans[i].clone(), i as Latency);
        }
        Self {
            spans,
            proc_ests,
            proc_elapses,
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
