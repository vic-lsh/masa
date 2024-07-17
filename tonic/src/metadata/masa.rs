use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

type Timestamp = u64;
type Latency = u64;
type Span = String;

/// Represent Masa Information.
#[derive(Serialize, Deserialize, Debug)]
pub struct Masainfo {
    start_at: Timestamp,
    deadline: Timestamp,
    graph: Graph,
}

impl Masainfo {
    /// Create a new Masainfo.
    pub fn new(start_at: Timestamp, deadline: Timestamp, graph: Graph) -> Self {
        Self {
            start_at,
            deadline,
            graph,
        }
    }

    /// Create a new Masainfo from JSON.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    /// Convert Masainfo to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }
}

/// Represent a call graph.
#[derive(Serialize, Deserialize, Debug)]
pub struct Graph {
    spans: Vec<Span>,
    proc_ests: HashMap<Span, Latency>,
    proc_elapses: HashMap<Span, Latency>,
}

impl Graph {
    /// Create a default Graph.
    pub fn default() -> Self {
        Self {
            spans: Vec::new(),
            proc_ests: HashMap::new(),
            proc_elapses: HashMap::new(),
        }
    }

    /// Create a new Graph.
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

    /// Create a new Graph from JSON.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    /// Convert Graph to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }
}
