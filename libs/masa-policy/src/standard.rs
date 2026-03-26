use std::sync::Arc;
use std::task::Poll;
use tonic_core::{CowGrpcMethod, GrpcMethod, Request, Status};

use crate::ac::AcHandler;
use crate::base::BaseHookState;
use crate::context_ext::MasaRequestExt;
use crate::predictive_overlay::{PredictiveOverlay, PredictiveOverlayChild, PredictiveOverlayServer};
use masa_core::ContextBuilder;
use tonic_core::masa_ext::resolve_method_name_from_request;
use tonic_core::masa_ext::{ClientHooks, Hooks, ParentHooks, ServerHooks};
use tonic_core::Response;

#[derive(Debug)]
/// Standard Masa hooks implementation shared by all scheduling policies
/// (sched_fifo, sched_slo, sched_tailclipper, sched_pred). The actual scheduling
/// differences are handled by the tokio runtime and, when `est` is enabled, the
/// `PredictiveOverlay` (deadline tightening, reprioritization, predictive AC).
#[allow(dead_code)]
pub struct StandardHooks;

impl Hooks for StandardHooks {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
pub struct ServerContext {
    predictive: PredictiveOverlayServer,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            predictive: PredictiveOverlayServer::new(),
        }
    }
}

#[derive(Debug)]
pub struct ParentContext {
    base: BaseHookState,
    predictive: PredictiveOverlay,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        let base = BaseHookState::new(method, req);

        Self {
            predictive: PredictiveOverlay::new(&base.resolved_method, &_server_ctx.predictive),
            base,
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.base.check_guards()?;
        self.predictive.before_poll(&self.base.ctx);
        self.base.track_poll();
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        self.base.check_guards_status()?;

        let child_method_name = resolve_method_name_from_request(child_method, request);
        self.base
            .ac
            .check_outbound(&child_method_name, &self.base.ctx)?;

        child_ctx.set_method_name(child_method_name.clone());

        let builder = ContextBuilder::from(&self.base.ctx)
            .tokens(self.base.ac.remaining_tokens());

        let (_deadline, _prio_hint, builder) = self.predictive.before_child_rpc(
            &self.base.ctx,
            &child_method_name,
            &mut child_ctx.predictive,
            request,
            builder,
            || self.base.slo_abort.issue_error(),
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
        self.base.q_lat_tracker.track_child_response(response);

        self.predictive
            .after_child_rpc(&self.base.ctx, response, &child_ctx.predictive)?;

        if let Some(child) = child_ctx.child_method_name {
            self.base.update_after_child(&child, response);
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.predictive.after_poll(poll);
        self.base.check_pending_guards(poll)
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        self.predictive.finalize(&self.base.ctx, result);
        self.base.finalize(result);
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {
    pub child_method_name: Option<CowGrpcMethod>,
    predictive: PredictiveOverlayChild,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            predictive: PredictiveOverlayChild::new(),
        }
    }
}

impl ChildContext {
    pub fn set_method_name(&mut self, name: CowGrpcMethod) {
        self.child_method_name = Some(name);
    }
}

#[cfg(test)]
mod tests {
    crate::generate_slo_abort_test!(ParentContext, ServerContext, ChildContext);

    #[cfg(feature = "est")]
    mod est_tests {
        use super::super::{ChildContext, ParentContext, ServerContext};
        use crate::context_ext::MASA_CONTEXT_HEADER;
        use crate::est::estimator::ParentToChildId;
        use crate::est::state::EstServerState;
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
            use tonic_core::masa_ext::{
                METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER,
            };

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
            assert!(child_ctx.predictive.est.parent_to_child_id.is_some());
            assert!(child_ctx.predictive.est.server.is_some());

            // Verify registry has IDs
            let registry = MethodRegistry::global();
            let parent_id = registry.get_or_register_method("IntegrationService", "ParentMethod");
            let child_id = registry.get_or_register_method("IntegrationService", "ChildMethod");

            assert_eq!(
                child_ctx.predictive.est.parent_to_child_id.clone().unwrap().parent_id,
                parent_id
            );
            assert_eq!(
                child_ctx.predictive.est.parent_to_child_id.clone().unwrap().child_id,
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
            let shed = parent_ctx.predictive.est.admission_check(&parent_ctx.base.ctx, 0);
            assert!(!shed, "should admit when plenty of time remains");
        }
    }
}
