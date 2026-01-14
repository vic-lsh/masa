use serde::{Deserialize, Serialize};

use crate::{Api, Latency, PriorityHint, RequestId, Timestamp};

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
    prio_hint: PriorityHint,
    frontend_elapse: Option<u64>,
}

pub struct ContextBuilder {
    api: Api,
    request_id: RequestId,
    slo: Latency,
    start_at: Timestamp,
    deadline: Timestamp,
    prio_hint: Option<PriorityHint>,
    frontend_elapse: Option<u64>,
}

impl ContextBuilder {
    pub fn new(api: impl Into<Api>, request_id: RequestId) -> Self {
        Self {
            api: api.into(),
            request_id,
            slo: 0,
            start_at: 0,
            deadline: 0,
            prio_hint: None,
            frontend_elapse: None,
        }
    }

    pub fn from(ctx: &Context) -> Self {
        Self {
            api: ctx.api.clone(),
            request_id: ctx.request_id,
            slo: ctx.slo,
            start_at: ctx.start_at,
            deadline: ctx.deadline,
            prio_hint: Some(ctx.prio_hint),
            frontend_elapse: ctx.frontend_elapse,
        }
    }

    pub fn slo(mut self, slo: Latency) -> Self {
        self.slo = slo;
        self
    }

    pub fn start_at(mut self, start_at: Timestamp) -> Self {
        self.start_at = start_at;
        self
    }

    pub fn deadline(mut self, deadline: Timestamp) -> Self {
        self.deadline = deadline;
        self
    }

    pub fn prio_hint(mut self, prio_hint: PriorityHint) -> Self {
        self.prio_hint = Some(prio_hint);
        self
    }

    pub fn frontend_elapse(mut self, elapse: u64) -> Self {
        self.frontend_elapse = Some(elapse);
        self
    }

    pub fn build(self) -> Context {
        Context {
            api: self.api,
            request_id: self.request_id,
            slo: self.slo,
            start_at: self.start_at,
            deadline: self.deadline,
            prio_hint: self.prio_hint.unwrap_or(PriorityHint::new(self.deadline)),
            frontend_elapse: self.frontend_elapse,
        }
    }
}

impl Context {
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

    pub fn prio_hint(&self) -> PriorityHint {
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
