use crate::{Code, Response, Status};
use masa_core::{time_now, Context, EARLY_RETURN};
#[cfg(feature = "trace-queue")]
use masa_core::QueueLatencies;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "trace-queue")]
use std::sync::atomic::AtomicU64;
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

#[cfg(feature = "trace-queue")]
#[derive(Debug)]
pub(crate) struct QueueLatencyTracker {
    initial_q_lat: AtomicU64,
    resume_q_lat: AtomicU64,
    is_first_poll: AtomicBool,
}

#[cfg(feature = "trace-queue")]
impl Default for QueueLatencyTracker {
    fn default() -> Self {
        Self {
            initial_q_lat: AtomicU64::new(0),
            resume_q_lat: AtomicU64::new(0),
            is_first_poll: AtomicBool::new(true),
        }
    }
}

#[cfg(feature = "trace-queue")]
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
            use super::MasaResponseExt;
            if let Some(ctx) = resp.get_masa_context() {
                if let Some(ql) = ctx.queue_latencies {
                    self.initial_q_lat.fetch_add(ql.initial, Ordering::AcqRel);
                    self.resume_q_lat.fetch_add(ql.resume, Ordering::AcqRel);
                }
            }
        }
    }

    pub(crate) fn inject_context_metadata<T>(
        &self,
        ctx: &Context,
        result: &mut Result<Response<T>, Status>,
    ) {
        use super::{MasaResponseExt, MasaStatusExt};

        let mut ctx = ctx.clone();

        let initial = self.initial_q_lat.load(Ordering::Acquire);
        let resume = self.resume_q_lat.load(Ordering::Acquire);
        ctx.queue_latencies = Some(QueueLatencies { initial, resume });

        match result {
            Ok(resp) => resp.set_masa_context(&ctx),
            Err(status) => status.set_masa_context(&ctx),
        };
    }
}

#[cfg(not(feature = "trace-queue"))]
#[derive(Debug, Default)]
pub(crate) struct QueueLatencyTracker;

#[cfg(not(feature = "trace-queue"))]
impl QueueLatencyTracker {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn track_poll(&self) {}

    pub(crate) fn track_child_response<T>(&self, _response: &Result<Response<T>, Status>) {}

    pub(crate) fn inject_context_metadata<T>(&self, _ctx: &Context, _result: &mut Result<Response<T>, Status>) {}
}
