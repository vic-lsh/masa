use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{Api, Latency, RequestClass, RequestId, TestId, Timestamp};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "duration")]
pub enum FutureSpan {
    #[serde(rename = "compute")]
    Compute(u64),
    #[serde(rename = "block")]
    Block(u64),
    #[serde(rename = "queueing")]
    Queueing(u64),
}

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    api: Api,
    test_id: TestId,
    request_id: RequestId,
    slo: Latency,
    request_class: RequestClass,
    start_at: Timestamp,
    deadline: Timestamp,
    frontend_elapse: Option<u64>,
}

impl Context {
    /// Create a new Masa context.
    pub fn new(
        api: Api,
        test_id: TestId,
        request_id: RequestId,
        slo: Latency,
        request_class: RequestClass,
        start_at: Timestamp,
        deadline: Timestamp,
    ) -> Self {
        Self {
            api,
            test_id,
            request_id,
            slo,
            request_class,
            start_at,
            deadline,
            frontend_elapse: None,
        }
    }

    /// Get the API.
    pub fn api(&self) -> &Api {
        &self.api
    }

    /// Get the test ID.
    pub fn test_id(&self) -> TestId {
        self.test_id
    }

    /// Get the request ID.
    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Get the SLO.
    pub fn slo(&self) -> Latency {
        self.slo
    }

    /// Get the request class.
    pub fn request_class(&self) -> RequestClass {
        self.request_class
    }

    /// Get the start timestamp.
    pub fn start_at(&self) -> Timestamp {
        self.start_at
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
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
