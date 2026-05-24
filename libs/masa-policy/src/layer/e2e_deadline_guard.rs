// E2E deadline guard layer — rejects requests that have exceeded their
// end-to-end SLO deadline, avoiding wasteful compute on responses that will
// miss their SLO regardless.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::task::Poll;

use masa_core::{time_now, Context, ABORT_SLO};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::{ChildRpcContext, Layer, LayerChild, LayerServer};

// ── Core Handler ──────────────────────────────────────────────────────

#[derive(Debug)]
struct SloAbortHandler {
    will_abort: AtomicBool,
    rpc: CowGrpcMethod,
    last_child: Mutex<Option<CowGrpcMethod>>,
}

impl Default for SloAbortHandler {
    fn default() -> Self {
        Self {
            will_abort: AtomicBool::new(false),
            rpc: CowGrpcMethod::new("", ""),
            last_child: Mutex::new(None),
        }
    }
}

impl SloAbortHandler {
    fn new(rpc: CowGrpcMethod) -> Self {
        Self {
            will_abort: AtomicBool::new(false),
            rpc,
            last_child: Mutex::new(None),
        }
    }

    fn set_last_child(&self, child: CowGrpcMethod) {
        if let Ok(mut last) = self.last_child.lock() {
            *last = Some(child);
        }
    }

    fn check(&self, ctx: &Context) -> bool {
        if !ABORT_SLO {
            return false;
        }

        // e2e_deadline=0 means no SLO was set (e.g. health-check pings); never early-return.
        // Use e2e_deadline (gateway_entry + slo) rather than ctx.deadline() so that policies
        // like pred_sched that tighten the per-hop deadline for scheduling purposes do not
        // cause premature early-returns — ER fires only at the actual end-to-end SLO boundary.
        let e2e_deadline = ctx.e2e_deadline();
        if e2e_deadline == 0 {
            return false;
        }

        if self.will_abort.load(Ordering::Relaxed) {
            return true;
        }

        let deadline = ctx.deadline();
        if deadline == 0 {
            return false;
        }

        let now = time_now();
        let should_early_return = now >= e2e_deadline;

        if should_early_return {
            // We use compare_exchange_weak to ensure we only log or trigger side effects once if needed,
            // though in this simple implementation it just sets the flag.
            let _ = self.will_abort.compare_exchange_weak(
                false,
                true,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        should_early_return
    }

    fn issue_error(&self) -> Status {
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

// ── Server ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct E2eDeadlineGuardServer;

impl LayerServer for E2eDeadlineGuardServer {
    fn new() -> Self {
        Self
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct E2eDeadlineGuardLayer {
    handler: SloAbortHandler,
}

impl Layer for E2eDeadlineGuardLayer {
    type Server = E2eDeadlineGuardServer;
    type Child = E2eDeadlineGuardChild;

    fn new(method: &CowGrpcMethod, _server: &E2eDeadlineGuardServer, _ctx: &mut Context) -> Self {
        Self {
            handler: SloAbortHandler::new(method.clone()),
        }
    }

    #[inline]
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        if self.handler.check(ctx) {
            return Err(Err(self.handler.issue_error()));
        }
        Ok(())
    }

    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        _child_ctx: &mut E2eDeadlineGuardChild,
        _request: &mut tonic_core::Request<T>,
        _child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        if self.handler.check(ctx) {
            return Err(self.handler.issue_error());
        }
        Ok(())
    }

    #[inline]
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        child_method: &CowGrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        _child_ctx: &E2eDeadlineGuardChild,
    ) -> Result<(), Status> {
        self.handler.set_last_child(child_method.clone());
        Ok(())
    }

    #[inline]
    fn after_poll<Ret>(
        &self,
        ctx: &Context,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.handler.check(ctx) {
                return Err(Err(self.handler.issue_error()));
            }
        }
        Ok(())
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct E2eDeadlineGuardChild;

impl LayerChild for E2eDeadlineGuardChild {
    fn new() -> Self {
        Self
    }
}

// ── Test macro ──────────────────────────────────────────────────────────

#[cfg(test)]
#[macro_export]
macro_rules! generate_abort_slo_test {
    ($ParentContext:ident, $ServerContext:ident, $ChildContext:ident) => {
        #[test]
        #[cfg(feature = "abort_slo")]
        fn test_early_return_tracking() {
            use super::{$ChildContext, $ParentContext, $ServerContext};
            use crate::context_ext::MASA_CONTEXT_HEADER;
            use masa_core::{time_now, ContextBuilder};
            use masa_tonic_core::{ClientHooks, ParentHooks, ServerHooks};
            use std::sync::Arc;
            use tonic_core::{GrpcMethod, Request, Response, Status};

            // Create a context with an e2e SLO deadline in the past.
            // SloAbortHandler now uses e2e_deadline (gateway_entry + slo) for the ER check,
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

            assert_eq!(err.code(), tonic_core::Code::DeadlineExceeded);
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
            child_ctx.set_method_name(tonic_core::CowGrpcMethod::new(
                "test.Service",
                "ChildMethod",
            ));

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
