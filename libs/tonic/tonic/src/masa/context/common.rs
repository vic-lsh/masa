use crate::{body::BoxBody, Code, Response, Status};
use masa_core::{time_now, Context, EARLY_RETURN};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug)]
pub(crate) struct EarlyReturnHandler {
    will_early_return: AtomicBool,
    service: &'static str,
    method: String,
    last_child: Mutex<Option<String>>,
}

impl Default for EarlyReturnHandler {
    fn default() -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            service: "",
            method: String::new(),
            last_child: Mutex::new(None),
        }
    }
}

impl EarlyReturnHandler {
    pub(crate) fn new(service: &'static str, method: String) -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            service,
            method,
            last_child: Mutex::new(None),
        }
    }

    pub(crate) fn set_last_child(&self, child: String) {
        if let Ok(mut last) = self.last_child.lock() {
            *last = Some(child);
        }
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

    pub(crate) fn issue_error(&self) -> Status {
        let last_child = self
            .last_child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| "None:None".to_string());
        Status::new(
            Code::DeadlineExceeded,
            format!(
                "/EarlyReturn:{}:{}|{}",
                self.service, self.method, last_child
            ),
        )
    }
}

#[derive(Debug)]
pub(crate) struct QueueLatencyTracker {
    initial_q_lat: AtomicU64,
    resume_q_lat: AtomicU64,
    is_first_poll: AtomicBool,
}

impl Default for QueueLatencyTracker {
    fn default() -> Self {
        Self {
            initial_q_lat: AtomicU64::new(0),
            resume_q_lat: AtomicU64::new(0),
            is_first_poll: AtomicBool::new(true),
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
            if self.is_first_poll.swap(false, Ordering::Relaxed) {
                self.initial_q_lat
                    .fetch_add(queue_latency, Ordering::AcqRel);
            } else {
                self.resume_q_lat.fetch_add(queue_latency, Ordering::AcqRel);
            }
        }
    }

    pub(crate) fn track_child_response<T>(&self, response: &Result<Response<T>, Status>) {
        if let Ok(resp) = response {
            if let Some(value) = resp
                .metadata()
                .get("x-queue-latency-initial")
                .or_else(|| resp.metadata().get("X-Queue-Latency-Initial"))
            {
                if let Ok(v) = value.to_str() {
                    if let Ok(parsed) = v.parse::<u64>() {
                        self.initial_q_lat.fetch_add(parsed, Ordering::AcqRel);
                    }
                }
            }

            if let Some(value) = resp
                .metadata()
                .get("x-queue-latency-resume")
                .or_else(|| resp.metadata().get("X-Queue-Latency-Resume"))
            {
                if let Ok(v) = value.to_str() {
                    if let Ok(parsed) = v.parse::<u64>() {
                        self.resume_q_lat.fetch_add(parsed, Ordering::AcqRel);
                    }
                }
            }
        }
    }

    pub(crate) fn inject_header(&self, response: &mut http::Response<BoxBody>) {
        let res_header = response.headers_mut();

        let initial = self.initial_q_lat.load(Ordering::Acquire);
        let resume = self.resume_q_lat.load(Ordering::Acquire);
        let total = initial + resume;

        if let Ok(header_val) = http::HeaderValue::from_str(&initial.to_string()) {
            res_header.insert("x-queue-latency-initial", header_val);
        }
        if let Ok(header_val) = http::HeaderValue::from_str(&resume.to_string()) {
            res_header.insert("x-queue-latency-resume", header_val);
        }
        if let Ok(header_val) = http::HeaderValue::from_str(&total.to_string()) {
            res_header.insert("x-queue-latency", header_val);
        }
    }
}
