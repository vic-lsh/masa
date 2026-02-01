use crate::{body::BoxBody, Code, Response, Status};
use masa::{time_now, Context, EarlyReturnEnabled, EarlyReturnDisabled};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub trait EarlyReturnHandlerTrait: Send + Sync + 'static {
    fn new(service: &'static str, method: String) -> Self;
    fn check(&self, ctx: &Context) -> bool;
    fn check_deadline(&self, deadline: u64) -> bool;
    fn issue_error(&self) -> Status;
}

#[derive(Debug)]
pub struct RealEarlyReturnHandler {
    will_early_return: AtomicBool,
    service: &'static str,
    method: String,
}

impl EarlyReturnHandlerTrait for RealEarlyReturnHandler {
    fn new(service: &'static str, method: String) -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            service,
            method,
        }
    }

    fn check(&self, ctx: &Context) -> bool {
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

    fn check_deadline(&self, deadline: u64) -> bool {
        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= deadline;

        if should_early_return {
            let _ = self.will_early_return.compare_exchange_weak(
                false,
                true,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        should_early_return
    }

    fn issue_error(&self) -> Status {
        Status::new(
            Code::DeadlineExceeded,
            format!("/EarlyReturn:{}:{}", self.service, self.method),
        )
    }
}

#[derive(Debug)]
pub struct NoopEarlyReturnHandler;

impl EarlyReturnHandlerTrait for NoopEarlyReturnHandler {
    fn new(_service: &'static str, _method: String) -> Self {
        Self
    }
    fn check(&self, _ctx: &Context) -> bool {
        false
    }
    fn check_deadline(&self, _deadline: u64) -> bool {
        false
    }
    fn issue_error(&self) -> Status {
        Status::ok("NoopEarlyReturnHandler")
    }
}

pub trait MapEarlyReturn {
    type Handler: EarlyReturnHandlerTrait;
}

impl MapEarlyReturn for EarlyReturnEnabled {
    type Handler = RealEarlyReturnHandler;
}

impl MapEarlyReturn for EarlyReturnDisabled {
    type Handler = NoopEarlyReturnHandler;
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