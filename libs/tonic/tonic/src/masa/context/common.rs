use crate::{body::BoxBody, Code, Response, Status};
use masa::{time_now, Context, EARLY_RETURN};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(crate) const EARLY_RETURN_HOP_HEADER: &str = "x-early-return-hop";

#[derive(Debug)]
pub(crate) struct EarlyReturnHandler {
    will_early_return: AtomicBool,
}

impl Default for EarlyReturnHandler {
    fn default() -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
        }
    }
}

impl EarlyReturnHandler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn check(&self, ctx: &Context) -> bool {
        if !EARLY_RETURN {
            return false;
        }

        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= ctx.deadline();

        if should_early_return {
            // We use compare_exchange_weak to ensure we only log or trigger side effects once if needed,
            // though in this simple implementation it just sets the flag.
            let _ = self.will_early_return.compare_exchange_weak(
                false,
                true,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        should_early_return
    }

    pub(crate) fn issue_error(&self, hop_label: &str) -> Status {
        let mut status = Status::new(Code::DeadlineExceeded, "/EarlyReturn");
        if let Ok(value) = hop_label.parse() {
            status.metadata_mut().insert(EARLY_RETURN_HOP_HEADER, value);
        }
        status
    }
}

#[derive(Debug)]
pub(crate) struct QueueLatencyTracker {
    q_lat: AtomicU64,
}

impl Default for QueueLatencyTracker {
    fn default() -> Self {
        Self {
            q_lat: AtomicU64::new(0),
        }
    }
}

impl QueueLatencyTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn track_poll(&self) {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        if queue_latency > 0 {
            self.q_lat.fetch_add(queue_latency, Ordering::AcqRel);
        }
    }

    pub(crate) fn track_child_response<T>(&self, response: &Result<Response<T>, Status>) {
        if let Ok(resp) = response {
            if let Some(value) = resp
                .metadata()
                .get("x-queue-latency")
                .or_else(|| resp.metadata().get("X-Queue-Latency"))
            {
                if let Ok(v) = value.to_str() {
                    if let Ok(parsed) = v.parse::<u64>() {
                        self.q_lat.fetch_add(parsed, Ordering::AcqRel);
                    }
                }
            }
        }
    }

    pub(crate) fn inject_header(&self, response: &mut http::Response<BoxBody>) {
        let res_header = response.headers_mut();
        let total = self.q_lat.load(Ordering::Acquire).to_string();
        if let Ok(header_val) = http::HeaderValue::from_str(&total) {
            res_header.insert("x-queue-latency", header_val);
        }
    }
}
