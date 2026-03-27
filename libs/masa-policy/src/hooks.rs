// Masa hooks implementation.
//
// `PolicyHooks` is the single concrete `Hooks` implementation used by all
// scheduling policies (sched_fifo, sched_slo, sched_tailclipper, sched_pred).
// The actual scheduling differences are handled by the tokio runtime and,
// when enabled, the active overlay (predictive or rajomon).

use std::sync::Arc;
use std::task::Poll;

use crate::context_ext::{read_context, MasaRequestExt};
use crate::overlay::{
    ActiveOverlay, ActiveOverlayChild, ActiveOverlayServer, ChildRpcContext, Overlay, OverlayChild,
    OverlayServer,
};
use crate::slo_abort::SloAbortHandler;
use masa_core::{Context, ContextBuilder};
use tonic_core::masa_ext::resolve_method_name_from_http;
use tonic_core::masa_ext::resolve_method_name_from_request;
use tonic_core::masa_ext::{ClientHooks, Hooks, ParentHooks, ServerHooks};
use tonic_core::{CowGrpcMethod, GrpcMethod, Request, Response, Status};

#[derive(Debug)]
#[allow(dead_code)]
pub struct PolicyHooks;

impl Hooks for PolicyHooks {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
pub struct ServerContext {
    overlay: ActiveOverlayServer,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            overlay: ActiveOverlayServer::new(),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {
    ctx: Context,
    resolved_method: CowGrpcMethod,
    q_lat_tracker: QueueLatencyTracker,
    slo_abort: SloAbortHandler,
    pub(crate) overlay: ActiveOverlay,
}

impl ParentContext {
    fn check_guards<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.slo_abort.check(&self.ctx) {
            return Err(Err(self.slo_abort.issue_error()));
        }
        Ok(())
    }

    fn check_guards_status(&self) -> Result<(), Status> {
        self.check_guards::<()>().map_err(|e| e.unwrap_err())
    }

    #[cfg(feature = "est")]
    pub(crate) fn ctx(&self) -> &Context {
        &self.ctx
    }
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        let mut ctx = read_context(req);
        let resolved_method = resolve_method_name_from_http(method, req);

        let overlay = ActiveOverlay::new(&resolved_method, &server_ctx.overlay, &mut ctx);
        let slo_abort = SloAbortHandler::new(resolved_method.clone());
        let q_lat_tracker = QueueLatencyTracker::new();

        Self {
            ctx,
            resolved_method,
            q_lat_tracker,
            slo_abort,
            overlay,
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.check_guards()?;
        self.overlay.before_poll(&self.ctx)?;
        self.q_lat_tracker.track_poll();
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        self.check_guards_status()?;

        let child_method_name = resolve_method_name_from_request(child_method, request);
        child_ctx.set_method_name(child_method_name.clone());

        let builder = ContextBuilder::from(&self.ctx);

        let ChildRpcContext { builder, .. } = self.overlay.before_child_rpc(
            &self.ctx,
            &child_method_name,
            &mut child_ctx.overlay,
            request,
            builder,
            || self.slo_abort.issue_error(),
        )?;

        let child_recv_ctx = builder.build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        self.q_lat_tracker.track_child_response(response);

        let child_method = child_ctx.child_method_name.as_ref();

        if let Some(child_method) = child_method {
            self.overlay.after_child_rpc(
                &self.ctx,
                child_method,
                response,
                &child_ctx.overlay,
            )?;
            self.slo_abort.set_last_child(child_method.clone());
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.overlay.after_poll(poll);
        if let Poll::Pending = poll {
            self.check_guards()?;
        }
        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        self.overlay.finalize(&self.ctx, result);
        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {
    pub child_method_name: Option<CowGrpcMethod>,
    overlay: ActiveOverlayChild,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            overlay: ActiveOverlayChild::new(),
        }
    }
}

impl ChildContext {
    pub fn set_method_name(&mut self, name: CowGrpcMethod) {
        self.child_method_name = Some(name);
    }
}

// ── Queue Latency Tracker ───────────────────────────────────────────────

#[cfg(feature = "trace-queue")]
use masa_core::QueueLatencies;
#[cfg(feature = "trace-queue")]
use std::sync::atomic::AtomicU64;
#[cfg(feature = "trace-queue")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "trace-queue")]
#[derive(Debug)]
struct QueueLatencyTracker {
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
    fn new() -> Self {
        Self::default()
    }

    fn track_poll(&self) {
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

    fn track_child_response<T>(&self, response: &Result<Response<T>, Status>) {
        if let Ok(resp) = response {
            use crate::context_ext::MasaResponseExt;
            if let Some(ctx) = resp.get_masa_context() {
                if let Some(ql) = ctx.queue_latencies {
                    self.initial_q_lat.fetch_add(ql.initial, Ordering::AcqRel);
                    self.resume_q_lat.fetch_add(ql.resume, Ordering::AcqRel);
                }
            }
        }
    }

    fn inject_context_metadata<T>(
        &self,
        ctx: &Context,
        result: &mut Result<Response<T>, Status>,
    ) {
        use crate::context_ext::{MasaResponseExt, MasaStatusExt};

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
struct QueueLatencyTracker;

#[cfg(not(feature = "trace-queue"))]
impl QueueLatencyTracker {
    fn new() -> Self {
        Self
    }

    fn track_poll(&self) {}

    fn track_child_response<T>(&self, _response: &Result<Response<T>, Status>) {}

    fn inject_context_metadata<T>(
        &self,
        _ctx: &Context,
        _result: &mut Result<Response<T>, Status>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    crate::generate_slo_abort_test!(ParentContext, ServerContext, ChildContext);

    #[cfg(feature = "est")]
    mod est_tests {
        use super::super::{ChildContext, ParentContext, ServerContext};
        use crate::context_ext::MASA_CONTEXT_HEADER;
        use crate::overlay::predictive::est::estimator::ParentToChildId;
        use crate::overlay::predictive::est::state::EstServerState;
        use masa_core::{ContextBuilder, LatencyRms};
        use std::sync::Arc;
        use tonic_core::masa_ext::resolve_method_name_from_http;
        use tonic_core::masa_ext::{ClientHooks, ParentHooks, ServerHooks};
        use tonic_core::{GrpcMethod, Request, Response};

        #[test]
        fn test_server_context_rms_integration() {
            let est = EstServerState::<LatencyRms>::new();
            let method = ParentToChildId {
                parent_id: 1,
                child_id: 2,
            };
            let key = method.to_key();

            // Inject an estimator with a short update interval (2) for testing.
            // By default, LatencyRms has a large update interval (512), which makes testing hard.
            {
                est.est_child_latency.insert(key, LatencyRms::new(2));
            }

            // 1st track: sum_sq=100, count=1, since_update=1. No update yet.
            est.est_child_latency.track(key, 10);

            // Estimate uses cached RMS value (initially 0).
            let val = est.est_child_latency.get_estimate(key);
            assert_eq!(val, Some(0));

            // 2nd track: sum_sq=200, count=2, since_update=2. Update triggers.
            // RMS = sqrt( (10^2 + 10^2) / 2 ) = 10.
            est.est_child_latency.track(key, 10);

            let val = est.est_child_latency.get_estimate(key);
            assert_eq!(val, Some(10));

            // 3rd track: sum_sq=200+400=600, count=3, since_update=1. No update yet.
            est.est_child_latency.track(key, 20);
            let val = est.est_child_latency.get_estimate(key);
            assert_eq!(val, Some(10)); // Still 10

            // 4th track: sum_sq=600+400=1000, count=4, since_update=2. Update triggers.
            // RMS = sqrt( (100 + 100 + 400 + 400) / 4 ) = sqrt(250) ~ 15.
            est.est_child_latency.track(key, 20);
            let val = est.est_child_latency.get_estimate(key);
            // integer_sqrt(250) is 15 (15*15=225, 16*16=256)
            assert_eq!(val, Some(15));
        }

        #[test]
        fn test_resolve_method_name_from_http_with_overrides() {
            use http::HeaderValue;
            use tonic_core::masa_ext::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};

            let method = GrpcMethod::new("TestService", "TestMethod");
            let mut req = http::Request::new(());

            // 1. No overrides
            let resolved = resolve_method_name_from_http(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "TestMethod");

            // 2. Method override only
            req.headers_mut().insert(
                METHOD_NAME_OVERRIDE_HEADER,
                HeaderValue::from_static("OverriddenMethod"),
            );
            let resolved = resolve_method_name_from_http(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "OverriddenMethod");

            // 3. Method and Service override
            req.headers_mut().insert(
                SERVICE_NAME_OVERRIDE_HEADER,
                HeaderValue::from_static("OverriddenService"),
            );
            let resolved = resolve_method_name_from_http(method, &req);
            assert_eq!(resolved.service(), "OverriddenService");
            assert_eq!(resolved.method(), "OverriddenMethod");
        }

        #[test]
        fn test_resolve_method_name_from_request_with_overrides() {
            use tonic_core::masa_ext::{
                resolve_method_name_from_request, METHOD_NAME_OVERRIDE_HEADER,
                SERVICE_NAME_OVERRIDE_HEADER,
            };
            use tonic_core::metadata::MetadataValue;

            let method = GrpcMethod::new("TestService", "TestMethod");
            let mut req = Request::new(());

            // 1. No overrides
            let resolved = resolve_method_name_from_request(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "TestMethod");

            // 2. Method override only
            req.metadata_mut().insert(
                METHOD_NAME_OVERRIDE_HEADER,
                MetadataValue::from_static("OverriddenMethod"),
            );
            let resolved = resolve_method_name_from_request(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "OverriddenMethod");

            // 3. Method and Service override
            req.metadata_mut().insert(
                SERVICE_NAME_OVERRIDE_HEADER,
                MetadataValue::from_static("OverriddenService"),
            );
            let resolved = resolve_method_name_from_request(method, &req);
            assert_eq!(resolved.service(), "OverriddenService");
            assert_eq!(resolved.method(), "OverriddenMethod");
        }

        #[test]
        fn test_local_deadline_policy_integration() {
            use crate::MethodRegistry;

            // 1. Setup Server Context
            let server_ctx = Arc::new(ServerContext::new("IntegrationService"));

            // 2. Prepare Parent Request
            let method = GrpcMethod::new("IntegrationService", "ParentMethod");
            let now = masa_core::time_now();
            let slo_us = 100_000u64; // 100ms SLO
            let deadline = now + slo_us;
            let ctx = ContextBuilder::new("IntegrationService", 123)
                .slo(slo_us)
                .gateway_entry(now)
                .deadline(deadline)
                .build();

            let req = http::Request::builder()
                .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
                .body(())
                .unwrap();

            // 3. Begin Parent Context (registers ParentMethod)
            let parent_ctx = ParentContext::begin(method, &req, server_ctx.clone());

            // 4. Before Child RPC (registers ChildMethod)
            let child_method = GrpcMethod::new("IntegrationService", "ChildMethod");
            let mut child_req = Request::new(());
            let mut child_ctx = ChildContext::new(child_method, &child_req);

            let _ = parent_ctx
                .before_child_rpc(child_method, &mut child_req, &mut child_ctx)
                .unwrap();

            // Verify child context has ID and Server
            assert!(child_ctx.overlay.est.parent_to_child_id.is_some());
            assert!(child_ctx.overlay.est.server.is_some());

            // Verify registry has IDs
            let registry = MethodRegistry::global();
            let parent_id = registry.get_or_register_method("IntegrationService", "ParentMethod");
            let child_id = registry.get_or_register_method("IntegrationService", "ChildMethod");

            assert_eq!(
                child_ctx
                    .overlay
                    .est
                    .parent_to_child_id
                    .clone()
                    .unwrap()
                    .parent_id,
                parent_id
            );
            assert_eq!(
                child_ctx
                    .overlay
                    .est
                    .parent_to_child_id
                    .clone()
                    .unwrap()
                    .child_id,
                child_id
            );

            // 5. Simulate Child Response
            let mut response = Ok(Response::new(()));
            let _ = parent_ctx
                .after_child_rpc(child_method, &mut response, child_ctx)
                .unwrap();

            // 6. Track Latencies
            let mut response_result = Ok(Response::new(()));
            parent_ctx.finalize_before_serialization(&mut response_result);

            // 7. Verify registry names
            let (p_s, p_m) = registry.get_method_name(parent_id).unwrap();
            assert_eq!(p_s, "IntegrationService");
            assert_eq!(p_m, "ParentMethod");
        }

        /// Verify that `admission_check` uses the floor-based check.
        /// The floor check should admit when there is plenty of time left (no shed).
        #[test]
        fn test_admission_check_floor_based_admits_with_budget() {
            let server_ctx = Arc::new(ServerContext::new("FloorService"));

            // Generous deadline: 100ms from now.
            let slo_us = 100_000u64;
            let now = masa_core::time_now();
            let deadline = now + slo_us;
            let ctx = ContextBuilder::new("FloorService", 42)
                .slo(slo_us)
                .gateway_entry(now)
                .deadline(deadline)
                .build();

            let req = http::Request::builder()
                .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
                .body(())
                .unwrap();

            let method = GrpcMethod::new("FloorService", "ParentMethod");
            let parent_ctx = ParentContext::begin(method, &req, server_ctx.clone());

            // With est_remaining_floor = 0, the floor check is: time_now > e2e_deadline - 0 = e2e_deadline.
            // Since e2e_deadline is 100ms in the future, this should NOT shed.
            let shed = parent_ctx
                .overlay
                .est
                .admission_check(parent_ctx.ctx(), 0);
            assert!(!shed, "should admit when plenty of time remains");
        }
    }
}
