use serde::{Deserialize, Serialize};

use crate::{GraphID, RequestID, Timestamp};

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    graph_id: GraphID,
    request_id: RequestID,
    deadline: Timestamp,
    send_at: Timestamp,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        graph_id: GraphID,
        request_id: RequestID,
        deadline: Timestamp,
        send_at: Timestamp,
    ) -> Self {
        Self {
            graph_id,
            request_id,
            deadline,
            send_at,
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

    /// Get the send timestamp.
    pub fn send_at(&self) -> Timestamp {
        self.send_at
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
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
