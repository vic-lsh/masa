#[cfg(feature = "trace_queue_latency")]
use std::collections::HashMap;
use std::fmt;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::{Api, Latency, PriorityHint, RequestId, Timestamp};

pub const MISSING_CONTEXT_HEADER_MESSAGE: &str =
    "missing MASA context header `ctx`; MASA-enabled services require clients to attach context via MASA context helpers";

pub fn invalid_context_header_metadata_message(error: impl fmt::Display) -> String {
    format!(
        "invalid MASA context header `{}`: invalid ASCII/metadata; MASA-enabled services require clients to attach context via MASA context helpers: {}",
        crate::MASA_CONTEXT_HEADER,
        error
    )
}

fn invalid_context_header_base64_message(error: impl fmt::Display) -> String {
    format!(
        "invalid MASA context header `{}`: invalid base64; MASA-enabled services require clients to attach context via MASA context helpers: {}",
        crate::MASA_CONTEXT_HEADER,
        error
    )
}

fn invalid_context_header_bincode_message(error: impl fmt::Display) -> String {
    format!(
        "invalid MASA context header `{}`: invalid bincode payload; MASA-enabled services require clients to attach context via MASA context helpers: {}",
        crate::MASA_CONTEXT_HEADER,
        error
    )
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestContext {
    pub api: Api,
    pub request_id: RequestId,
    pub slo: Latency,
    pub gateway_entry: Timestamp,
    pub deadline: Timestamp,
    pub prio_hint: PriorityHint,
    pub frontend_elapse: Option<u64>,
}

#[cfg(feature = "trace_queue_latency")]
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct QueueLatencies {
    pub initial: u64,
    pub resume: u64,
    /// Queue length at each service when this request's task was first polled.
    /// Populated only when `trace_queue_latency` feature is enabled.
    // skip_serializing_if is intentionally omitted: bincode is positional and
    // skipping a field on serialization causes UnexpectedEof on deserialization.
    #[serde(default)]
    pub queue_lengths: HashMap<String, u64>,
}

#[cfg(feature = "trace_queue_latency")]
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct QueueContext {
    #[serde(default)]
    pub latencies: Option<QueueLatencies>,
}

#[cfg(feature = "estimator")]
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EstimatorResponse {
    pub compute_time_us: u64,
    #[serde(default)]
    pub accumulated_compute_us: u64,
    pub utilization: f32,
    pub max_downstream_util: f32,
    /// Number of early returns in the subtree (this hop + all children).
    #[serde(default)]
    pub early_return_count: u32,
    /// Whether any hop in this request's subtree (this hop or any descendant)
    /// tripped its local deadline under `signal_slack`. Saturated at 1 so a
    /// single user-facing request never counts as multiple events, regardless
    /// of how many hops it traversed. Stored as `u32` for forward-compat with
    /// any future weighted use; today consumers should treat it as a boolean
    /// (`> 0`).
    #[serde(default)]
    pub deadline_signal_count: u32,
}

#[cfg(feature = "estimator")]
pub type ResponseMeta = EstimatorResponse;

/// Identifies the root (ingress) RPC method. Transported over the wire as a
/// (service, method) pair so that method identity is stable across replicas.
#[cfg(feature = "estimator")]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct RootMethod {
    pub service: String,
    pub method: String,
}

#[cfg(feature = "estimator")]
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EstimatorContext {
    #[serde(default)]
    pub response: Option<EstimatorResponse>,
    #[serde(default)]
    pub hop_count: u8,
    #[serde(default)]
    pub root_method: Option<RootMethod>,
}

#[cfg(feature = "ac_rajomon")]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RajomonContext {
    pub tokens: u64,
}

#[cfg(feature = "ac_rajomon")]
impl Default for RajomonContext {
    fn default() -> Self {
        Self { tokens: 100 }
    }
}

/// Represent a Masa context.
///
/// Header serialization uses bincode, so the positional wire layout changes
/// with these feature-gated fields. Masa deployments assume all binaries are
/// built with the same feature set; invalid decodes panic immediately.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Context {
    // Core request lifecycle data.
    request: RequestContext,
    // Queue-latency telemetry.
    #[cfg(feature = "trace_queue_latency")]
    #[serde(default)]
    pub queue: QueueContext,
    // Estimation propagation and response metadata.
    #[cfg(feature = "estimator")]
    #[serde(default)]
    pub estimator: EstimatorContext,
    // Rajomon admission state.
    #[cfg(feature = "ac_rajomon")]
    #[serde(default)]
    pub rajomon: RajomonContext,
}

impl Default for Context {
    fn default() -> Self {
        ContextBuilder::new(Api::default(), 0).build()
    }
}

pub struct ContextBuilder {
    api: Api,
    request_id: RequestId,
    slo: Latency,
    gateway_entry: Timestamp,
    deadline: Timestamp,
    prio_hint: Option<PriorityHint>,
    frontend_elapse: Option<u64>,
    #[cfg(feature = "trace_queue_latency")]
    queue_latencies: Option<QueueLatencies>,
    #[cfg(feature = "estimator")]
    response: Option<EstimatorResponse>,
    #[cfg(feature = "estimator")]
    hop_count: u8,
    #[cfg(feature = "estimator")]
    root_method: Option<RootMethod>,
    #[cfg(feature = "ac_rajomon")]
    tokens: u64,
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
            #[cfg(feature = "trace_queue_latency")]
            queue_latencies: None,
            #[cfg(feature = "estimator")]
            response: None,
            #[cfg(feature = "estimator")]
            hop_count: 0,
            #[cfg(feature = "estimator")]
            root_method: None,
            #[cfg(feature = "ac_rajomon")]
            tokens: RajomonContext::default().tokens,
        }
    }

    pub fn from(ctx: &Context) -> Self {
        Self {
            api: ctx.request.api.clone(),
            request_id: ctx.request.request_id,
            slo: ctx.request.slo,
            gateway_entry: ctx.request.gateway_entry,
            deadline: ctx.request.deadline,
            prio_hint: Some(ctx.request.prio_hint),
            frontend_elapse: ctx.request.frontend_elapse,
            #[cfg(feature = "trace_queue_latency")]
            queue_latencies: ctx.queue.latencies.clone(),
            #[cfg(feature = "estimator")]
            response: ctx.estimator.response.clone(),
            #[cfg(feature = "estimator")]
            hop_count: ctx.estimator.hop_count,
            #[cfg(feature = "estimator")]
            root_method: ctx.estimator.root_method.clone(),
            #[cfg(feature = "ac_rajomon")]
            tokens: ctx.rajomon.tokens,
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

    #[cfg(feature = "trace_queue_latency")]
    pub fn queue_latencies(mut self, queue_latencies: QueueLatencies) -> Self {
        self.queue_latencies = Some(queue_latencies);
        self
    }

    #[cfg(feature = "estimator")]
    pub fn response_meta(mut self, meta: EstimatorResponse) -> Self {
        self.response = Some(meta);
        self
    }

    #[cfg(feature = "estimator")]
    pub fn hop_count(mut self, hop_count: u8) -> Self {
        self.hop_count = hop_count;
        self
    }

    #[cfg(feature = "ac_rajomon")]
    pub fn tokens(mut self, tokens: u64) -> Self {
        self.tokens = tokens;
        self
    }

    #[cfg(feature = "estimator")]
    pub fn root_method(mut self, root_method: RootMethod) -> Self {
        self.root_method = Some(root_method);
        self
    }

    pub fn build(self) -> Context {
        Context {
            request: RequestContext {
                api: self.api,
                request_id: self.request_id,
                slo: self.slo,
                gateway_entry: self.gateway_entry,
                deadline: self.deadline,
                prio_hint: self.prio_hint.unwrap_or_else(|| {
                    #[cfg(feature = "sched_tailclipper")]
                    {
                        PriorityHint::new(self.gateway_entry)
                    }
                    #[cfg(not(feature = "sched_tailclipper"))]
                    {
                        #[cfg(feature = "sched_pred")]
                        {
                            PriorityHint::new(self.deadline.saturating_sub(crate::time_now()))
                        }
                        #[cfg(not(feature = "sched_pred"))]
                        {
                            PriorityHint::new(self.deadline)
                        }
                    }
                }),
                frontend_elapse: self.frontend_elapse,
            },
            #[cfg(feature = "trace_queue_latency")]
            queue: QueueContext {
                latencies: self.queue_latencies,
            },
            #[cfg(feature = "estimator")]
            estimator: EstimatorContext {
                response: self.response,
                hop_count: self.hop_count,
                root_method: self.root_method,
            },
            #[cfg(feature = "ac_rajomon")]
            rajomon: RajomonContext {
                tokens: self.tokens,
            },
        }
    }
}

impl Context {
    /// Get the API.
    pub fn api(&self) -> &Api {
        &self.request.api
    }

    /// Get the request ID.
    pub fn request_id(&self) -> RequestId {
        self.request.request_id
    }

    /// Get the SLO.
    pub fn slo(&self) -> Latency {
        self.request.slo
    }

    /// Get the start timestamp.
    pub fn gateway_entry(&self) -> Timestamp {
        self.request.gateway_entry
    }

    /// Get the deadline.
    pub fn deadline(&self) -> Timestamp {
        self.request.deadline
    }

    /// Get the e2e deadline.
    pub fn e2e_deadline(&self) -> Timestamp {
        self.request.gateway_entry + self.request.slo
    }

    /// Get the tokens budget.
    #[cfg(feature = "ac_rajomon")]
    pub fn tokens(&self) -> u64 {
        self.rajomon.tokens
    }

    /// Consume tokens from the budget. Returns false if budget is insufficient.
    #[cfg(feature = "ac_rajomon")]
    pub fn consume_tokens(&mut self, amount: u64) -> bool {
        if self.rajomon.tokens >= amount {
            self.rajomon.tokens -= amount;
            true
        } else {
            false
        }
    }

    pub fn prio_hint(&self) -> PriorityHint {
        self.request.prio_hint
    }

    /// Get the frontend elapse time.
    pub fn frontend_elapse(&self) -> Option<u64> {
        self.request.frontend_elapse
    }

    /// Set the frontend elapse time.
    pub fn set_frontend_elapse(&mut self, elapse: u64) {
        self.request.frontend_elapse = Some(elapse);
    }

    /// Get request lifecycle data.
    pub fn request(&self) -> &RequestContext {
        &self.request
    }

    /// Get queue latency telemetry.
    #[cfg(feature = "trace_queue_latency")]
    pub fn queue_latencies(&self) -> Option<&QueueLatencies> {
        self.queue.latencies.as_ref()
    }

    /// Set queue latency telemetry.
    #[cfg(feature = "trace_queue_latency")]
    pub fn set_queue_latencies(&mut self, queue_latencies: QueueLatencies) {
        self.queue.latencies = Some(queue_latencies);
    }

    /// Get the response metadata.
    #[cfg(feature = "estimator")]
    pub fn response_meta(&self) -> Option<&EstimatorResponse> {
        self.estimator.response.as_ref()
    }

    /// Set the response metadata.
    #[cfg(feature = "estimator")]
    pub fn set_response_meta(&mut self, meta: EstimatorResponse) {
        self.estimator.response = Some(meta);
    }

    /// Get the hop count.
    #[cfg(feature = "estimator")]
    pub fn hop_count(&self) -> u8 {
        self.estimator.hop_count
    }

    /// Get the root API method (set at ingress, propagated unchanged).
    #[cfg(feature = "estimator")]
    pub fn root_method(&self) -> Option<&RootMethod> {
        self.estimator.root_method.as_ref()
    }

    /// Set the root API method at ingress.
    #[cfg(feature = "estimator")]
    pub fn set_root_method(&mut self, root_method: RootMethod) {
        self.estimator.root_method = Some(root_method);
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
        let bytes = BASE64
            .decode(s)
            .unwrap_or_else(|err| panic!("{}", invalid_context_header_base64_message(err)));
        bincode::deserialize(&bytes)
            .unwrap_or_else(|err| panic!("{}", invalid_context_header_bincode_message(err)))
    }

    /// Convert a Masa context to Base64 encoded bincode.
    pub fn to_header_string(&self) -> String {
        let bytes = bincode::serialize(&self).unwrap();
        BASE64.encode(bytes)
    }
}
