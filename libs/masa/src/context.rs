use serde::{Deserialize, Serialize};

use crate::{Api, Latency, RequestId, Timestamp};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "duration")]
pub enum FutureSpan {
    #[serde(rename = "compute")]
    Compute(u64),
    #[serde(rename = "local_block")]
    LocalBlock(u64),
    #[serde(rename = "child_block")]
    ChildBlock(u64),
    #[serde(rename = "queue")]
    Queueing(u64),
}

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    api: Api,
    request_id: RequestId,
    slo: Latency,
    start_at: Timestamp,
    deadline: Timestamp,
    prio_hint: Timestamp,
    frontend_elapse: Option<u64>,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        api: Api,
        request_id: RequestId,
        slo: Latency,
        start_at: Timestamp,
        deadline: Timestamp,
        prio_hint: Timestamp,
    ) -> Self {
        Self {
            api,
            request_id,
            slo,
            start_at,
            deadline,
            prio_hint,
            frontend_elapse: None,
        }
    }

    /// Get the API.
    pub fn api(&self) -> &Api {
        &self.api
    }

    /// Get the request ID.
    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Get the SLO.
    pub fn slo(&self) -> Latency {
        self.slo
    }

    /// Get the start timestamp.
    pub fn start_at(&self) -> Timestamp {
        self.start_at
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    pub fn prio_hint(&self) -> Timestamp {
        self.prio_hint
    }

    /// Get the frontend elapse time.
    pub fn frontend_elapse(&self) -> Option<u64> {
        self.frontend_elapse
    }

    /// Set the frontend elapse time.
    pub fn set_frontend_elapse(&mut self, elapse: u64) {
        self.frontend_elapse = Some(elapse);
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
