use crate::{Code, CowGrpcMethod, Response, Status};
#[cfg(feature = "trace-queue")]
use masa_core::QueueLatencies;
use masa_core::{time_now, Context, EARLY_RETURN};
#[cfg(feature = "trace-queue")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug)]
pub(crate) struct EarlyReturnHandler {
    will_early_return: AtomicBool,
    rpc: CowGrpcMethod,
    last_child: Mutex<Option<CowGrpcMethod>>,
}

impl Default for EarlyReturnHandler {
    fn default() -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            rpc: CowGrpcMethod::new("", ""),
            last_child: Mutex::new(None),
        }
    }
}

impl EarlyReturnHandler {
    pub(crate) fn new(rpc: CowGrpcMethod) -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            rpc,
            last_child: Mutex::new(None),
        }
    }

    pub(crate) fn set_last_child(&self, child: CowGrpcMethod) {
        if let Ok(mut last) = self.last_child.lock() {
            *last = Some(child);
        }
    }

    pub(crate) fn check(&self, ctx: &Context) -> bool {
        if !EARLY_RETURN {
            return false;
        }

        // e2e_deadline=0 means no SLO was set (e.g. health-check pings); never early-return.
        // Use e2e_deadline (gateway_entry + slo) rather than ctx.deadline() so that policies
        // like prio_local that tighten the per-hop deadline for scheduling purposes do not
        // cause premature early-returns — ER fires only at the actual end-to-end SLO boundary.
        let e2e_deadline = ctx.e2e_deadline();
        if e2e_deadline == 0 {
            return false;
        }

        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= e2e_deadline;

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
            .clone();

        let mut msg = format!(
            "/EarlyReturn?src={}::{}",
            self.rpc.service(),
            self.rpc.method()
        );

        if let Some(child) = last_child {
            msg.push_str(&format!(
                "?last_rpc={}::{}",
                child.service(),
                child.method()
            ));
        }

        Status::new(Code::DeadlineExceeded, msg)
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

    pub(crate) fn inject_context_metadata<T>(
        &self,
        _ctx: &Context,
        _result: &mut Result<Response<T>, Status>,
    ) {
    }
}

#[cfg(test)]
#[macro_export]
macro_rules! generate_early_return_test {
    ($ParentContext:ident, $ServerContext:ident, $ChildContext:ident) => {
        #[test]
        #[cfg(feature = "early")]
        fn test_early_return_tracking() {
            use super::{$ChildContext, $ParentContext, $ServerContext};
            use crate::masa::context::MASA_CONTEXT_HEADER;
            use crate::masa::context::{ClientHooks, ParentHooks, ServerHooks};
            use crate::{GrpcMethod, Request, Response, Status};
            use masa_core::{time_now, ContextBuilder};
            use std::sync::Arc;

            // Create a context with an e2e SLO deadline in the past.
            // EarlyReturnHandler now uses e2e_deadline (gateway_entry + slo) for the ER check,
            // so we must set both gateway_entry and slo such that their sum is in the past.
            let slo = 1_000_u64; // 1ms SLO
            let gateway_entry = time_now().saturating_sub(1_000_000); // entered 1s ago
            let ctx = ContextBuilder::new("test-service", 123)
                .slo(slo)
                .gateway_entry(gateway_entry)
                .deadline(gateway_entry + slo) // already expired
                .build();

            let req = http::Request::builder()
                .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
                .body(())
                .unwrap();

            let method = GrpcMethod::new("test.Service", "Method");
            let server_ctx = Arc::new($ServerContext::new("test-service"));

            // Initialize ParentContext
            let parent_ctx = $ParentContext::begin(method, &req, server_ctx);

            // Check before_poll (should trigger early return)
            let result = parent_ctx.before_poll::<()>();
            assert!(result.is_err(), "Expected early return error");

            let err = match result {
                Err(Err(status)) => status,
                _ => panic!("Expected Err(Err(Status))"),
            };

            assert_eq!(err.code(), crate::Code::DeadlineExceeded);
            // Verify error message format: /EarlyReturn?src=test.Service::Method
            let msg = err.message();
            assert!(
                msg.contains("/EarlyReturn"),
                "Message should contain /EarlyReturn: {}",
                msg
            );
            assert!(
                msg.contains("src=test.Service::Method"),
                "Message should contain src: {}",
                msg
            );

            // Update last child info manually (simulating a completed child call)
            let mut child_ctx = $ChildContext::new(method, &Request::new(()));
            child_ctx.set_method_name(crate::CowGrpcMethod::new("test.Service", "ChildMethod"));

            // This simulates a child RPC finishing
            let mut resp_result: Result<Response<()>, Status> = Ok(Response::new(()));
            let _ = parent_ctx.after_child_rpc(method, &mut resp_result, child_ctx);

            // Check before_poll again - should now include last_rpc info
            let result = parent_ctx.before_poll::<()>();
            let err = match result {
                Err(Err(status)) => status,
                _ => panic!("Expected Err(Err(Status))"),
            };

            let msg = err.message();
            assert!(
                msg.contains("last_rpc=test.Service::ChildMethod"),
                "Message should contain last_rpc: {}",
                msg
            );
        }
    };
}
