use serde::{Deserialize, Serialize};

use crate::{GraphID, RequestClass, RequestID, Timestamp};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    graph_id: GraphID,
    request_id: RequestID,
    deadline: Timestamp,
    latest_exec_at: Timestamp,
    request_class: RequestClass,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        graph_id: GraphID,
        request_id: RequestID,
        deadline: Timestamp,
        latest_exec_at: Timestamp,
        request_class: RequestClass,
    ) -> Self {
        Self {
            graph_id,
            request_id,
            deadline,
            latest_exec_at,
            request_class,
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

    // /// Get the send timestamp.
    // pub fn send_at(&self) -> Timestamp {
    //     self.send_at
    // }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    /// Get the latest execution timestamp.
    pub fn latest_exec_at(&self) -> Timestamp {
        self.latest_exec_at
    }

    /// Get the request class.
    pub fn request_class(&self) -> RequestClass {
        self.request_class
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
