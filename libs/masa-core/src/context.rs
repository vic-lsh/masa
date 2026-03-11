use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
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

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct QueueLatencies {
    pub initial: u64,
    pub resume: u64,
}

/// Represent a Masa context.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Context {
    api: Api,
    request_id: RequestId,
    slo: Latency,
    gateway_entry: Timestamp,
    deadline: Timestamp,
    prio_hint: PriorityHint,
    frontend_elapse: Option<u64>,
    #[serde(default)]
    pub queue_latencies: Option<QueueLatencies>,
    #[cfg(feature = "emp_admission")]
    #[serde(default)]
    emp_admitted: bool,
}

pub struct ContextBuilder {
    api: Api,
    request_id: RequestId,
    slo: Latency,
    gateway_entry: Timestamp,
    deadline: Timestamp,
    prio_hint: Option<PriorityHint>,
    frontend_elapse: Option<u64>,
    queue_latencies: Option<QueueLatencies>,
    #[cfg(feature = "emp_admission")]
    emp_admitted: bool,
}

impl ContextBuilder {
    pub fn new(api: impl Into<Api>, request_id: RequestId) -> Self {
        Self {
            api: api.into(),
            request_id,
            slo: 0,
            gateway_entry: 0,
            deadline: 0,
            prio_hint: None,
            frontend_elapse: None,
            queue_latencies: None,
            #[cfg(feature = "emp_admission")]
            emp_admitted: false,
        }
    }

    pub fn from(ctx: &Context) -> Self {
        Self {
            api: ctx.api.clone(),
            request_id: ctx.request_id,
            slo: ctx.slo,
            gateway_entry: ctx.gateway_entry,
            deadline: ctx.deadline,
            prio_hint: Some(ctx.prio_hint),
            frontend_elapse: ctx.frontend_elapse,
            queue_latencies: ctx.queue_latencies.clone(),
            #[cfg(feature = "emp_admission")]
            emp_admitted: ctx.emp_admitted,
        }
    }

    pub fn slo(mut self, slo: Latency) -> Self {
        self.slo = slo;
        self
    }

    pub fn gateway_entry(mut self, gateway_entry: Timestamp) -> Self {
        self.gateway_entry = gateway_entry;
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

    pub fn queue_latencies(mut self, queue_latencies: QueueLatencies) -> Self {
        self.queue_latencies = Some(queue_latencies);
        self
    }

    #[cfg(feature = "emp_admission")]
    pub fn emp_admitted(mut self, v: bool) -> Self {
        self.emp_admitted = v;
        self
    }

    pub fn build(self) -> Context {
        Context {
            api: self.api,
            request_id: self.request_id,
            slo: self.slo,
            gateway_entry: self.gateway_entry,
            deadline: self.deadline,
            prio_hint: self.prio_hint.unwrap_or(PriorityHint::new(self.deadline)),
            frontend_elapse: self.frontend_elapse,
            queue_latencies: self.queue_latencies,
            #[cfg(feature = "emp_admission")]
            emp_admitted: self.emp_admitted,
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
    pub fn gateway_entry(&self) -> Timestamp {
        self.gateway_entry
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    /// Get the e2e deadline.
    pub fn e2e_deadline(&self) -> Timestamp {
        self.gateway_entry + self.slo
    }

    pub fn prio_hint(&self) -> PriorityHint {
        self.prio_hint
    }

    /// Get the frontend elapse time.
    pub fn frontend_elapse(&self) -> Option<u64> {
        self.frontend_elapse
    }

    /// Returns true if this request was already admitted via emp_admission at an upstream hop.
    #[cfg(feature = "emp_admission")]
    pub fn emp_admitted(&self) -> bool {
        self.emp_admitted
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

    /// Create a new Masa context from Base64 encoded bincode.
    pub fn from_header_string(s: &str) -> Self {
        let bytes = BASE64.decode(s).unwrap();
        bincode::deserialize(&bytes).unwrap()
    }

    /// Convert a Masa context to Base64 encoded bincode.
    pub fn to_header_string(&self) -> String {
        let bytes = bincode::serialize(&self).unwrap();
        BASE64.encode(bytes)
    }
}
