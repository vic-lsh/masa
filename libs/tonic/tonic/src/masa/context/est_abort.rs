use crate::masa::MethodRegistry;
use crate::{GrpcMethod, Request, Response, Status};
use std::sync::Arc;
use std::task::Poll;

use super::base::BaseHookState;
use super::est::state::{is_early_return_response, EstChildState, EstRequestState, EstServerState};
use super::{ClientHooks, MasaHooks, MasaRequestExt, ParentHooks, ServerHooks};
use masa_core::{time_now, ContextBuilder, LatencyEstimator, PriorityHint};

use super::est::estimator::DefaultLatencyEstimator as LocalLatencyEstimator;

#[derive(Debug)]
/// This policy computes the deadline d of a child request as
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct EstAbort;

impl MasaHooks for EstAbort {
    type ServerContext = ServerContext<LocalLatencyEstimator>;
    type ChildContext = ChildContext<LocalLatencyEstimator>;
    type ParentContext = ParentContext<LocalLatencyEstimator>;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    pub(super) est: Arc<EstServerState<E>>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        Self {
            est: Arc::new(EstServerState::new()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    base: BaseHookState,
    est: EstRequestState<E>,
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext<E>, ServerContext<E>>
    for ParentContext<E>
{
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<E>>,
    ) -> Self {
        let base = BaseHookState::new(method, req);
        let resolved_method_id = MethodRegistry::global().get_or_register_method(
            base.resolved_method.service(),
            base.resolved_method.method(),
        );

        Self {
            base,
            est: EstRequestState::new(resolved_method_id, server_ctx.est.clone()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.base.check_guards()?;

        let remaining = self.base.ctx.deadline().saturating_sub(time_now());
        tokio::task::reprioritize(masa_core::PriorityHint::new(remaining));

        self.base.track_poll();
        self.est.start_compute_tracking();
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.est.stop_compute_tracking();
        self.base.check_pending_guards(poll)
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext<E>,
    ) -> Result<(), Status> {
        self.base.check_guards_status()?;

        let resolved_child_method = super::resolve_method_name_from_request(child_method, request);

        self.base
            .rajomon
            .check_outbound(&resolved_child_method, &self.base.ctx)?;

        let prepare_result = self
            .est
            .prepare_before_child_rpc(&self.base.ctx, &resolved_child_method, &mut child_ctx.est)
            .map_err(|_| self.base.slo_abort.issue_error())?;

        let deadline = self
            .base
            .ctx
            .deadline()
            .saturating_sub(prepare_result.est_remaining);
        // prio_hint = deadline (parent_deadline - est_remaining). Subtracting est_child caused
        // priority inversions under load: as est_child grew with queuing, newer requests got
        // tighter prio_hints than older ones computed with lower estimates, inverting FIFO order.
        // Using deadline alone gives FIFO-like ordering (since all requests share the same e2e
        // SLO deadline), while preserving path-aware early returns from the tightened deadline.
        let prio_hint = deadline;

        let child_recv_ctx = ContextBuilder::from(&self.base.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(prio_hint))
            .hop_count(self.base.ctx.hop_count().saturating_add(1))
            .tokens(self.base.rajomon.remaining_tokens())
            .build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext<E>,
    ) -> Result<(), Status> {
        self.base.q_lat_tracker.track_child_response(response);
        child_ctx.est.finalize(response);
        self.est
            .after_child_rpc(&self.base.ctx, response, &child_ctx.est);

        if let Some(child_method) = &child_ctx.est.child_method {
            self.base.update_after_child(child_method, response);
        }

        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.est.track_latencies();
        }
        self.est.inject_response_meta(&self.base.ctx, result);

        self.base.finalize(result);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    est: EstChildState<E>,
}

impl<E: LatencyEstimator + Default + 'static> ClientHooks for ChildContext<E> {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            est: EstChildState::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::resolve_method_name_from_http;
    use super::*;
    use masa_core::LatencyRms;

    #[test]
    fn test_server_context_rms_integration() {
        let ctx = ServerContext::<LatencyRms>::new("test_service");
        let method = super::super::est::estimator::ParentToChildId {
            parent_id: 1,
            child_id: 2,
        };
        let key = method.to_key();

        // Inject an estimator with a short update interval (2) for testing.
        // By default, LatencyRms has a large update interval (512), which makes testing hard.
        {
            ctx.est.est_child_latency.insert(key, LatencyRms::new(2));
        }

        // 1st track: sum_sq=100, count=1, since_update=1. No update yet.
        ctx.est.est_child_latency.track(key, 10);

        // Estimate uses cached RMS value (initially 0).
        let est = ctx.est.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(0));

        // 2nd track: sum_sq=200, count=2, since_update=2. Update triggers.
        // RMS = sqrt( (10^2 + 10^2) / 2 ) = 10.
        ctx.est.est_child_latency.track(key, 10);

        let est = ctx.est.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10));

        // 3rd track: sum_sq=200+400=600, count=3, since_update=1. No update yet.
        ctx.est.est_child_latency.track(key, 20);
        let est = ctx.est.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10)); // Still 10

        // 4th track: sum_sq=600+400=1000, count=4, since_update=2. Update triggers.
        // RMS = sqrt( (100 + 100 + 400 + 400) / 4 ) = sqrt(250) ≈ 15.
        ctx.est.est_child_latency.track(key, 20);
        let est = ctx.est.est_child_latency.get_estimate(key);
        // integer_sqrt(250) is 15 (15*15=225, 16*16=256)
        assert_eq!(est, Some(15));
    }

    #[test]
    fn test_resolve_method_name_from_http_with_overrides() {
        use crate::masa::context::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
        use http::HeaderValue;

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
        use crate::masa::context::resolve_method_name_from_request;
        use crate::masa::context::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
        use crate::metadata::MetadataValue;

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
        use crate::masa::context::MASA_CONTEXT_HEADER;
        use masa_core::ContextBuilder;

        // 1. Setup Server Context
        let server_ctx = Arc::new(ServerContext::<LatencyRms>::new("IntegrationService"));

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
        let parent_ctx = ParentContext::<LatencyRms>::begin(method, &req, server_ctx.clone());

        // 4. Before Child RPC (registers ChildMethod)
        let child_method = GrpcMethod::new("IntegrationService", "ChildMethod");
        let mut child_req = Request::new(());
        let mut child_ctx = ChildContext::<LatencyRms>::new(child_method, &child_req);

        let _ = parent_ctx
            .before_child_rpc(child_method, &mut child_req, &mut child_ctx)
            .unwrap();

        // Verify child context has ID and Server
        assert!(child_ctx.est.parent_to_child_id.is_some());
        assert!(child_ctx.est.server.is_some());

        // Verify registry has IDs
        let registry = MethodRegistry::global();
        let parent_id = registry.get_or_register_method("IntegrationService", "ParentMethod");
        let child_id = registry.get_or_register_method("IntegrationService", "ChildMethod");

        assert_eq!(
            child_ctx.est.parent_to_child_id.clone().unwrap().parent_id,
            parent_id
        );
        assert_eq!(
            child_ctx.est.parent_to_child_id.clone().unwrap().child_id,
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

    /// Verify that without `ac_est`, `admission_check` falls back to the floor-based check.
    /// The floor check should admit when there is plenty of time left (no shed).
    #[cfg(not(feature = "ac_est"))]
    #[test]
    fn test_admission_check_floor_based_admits_with_budget() {
        use crate::masa::context::MASA_CONTEXT_HEADER;
        use masa_core::ContextBuilder;

        let server_ctx = Arc::new(ServerContext::<LatencyRms>::new("FloorService"));

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
        let parent_ctx = ParentContext::<LatencyRms>::begin(method, &req, server_ctx.clone());

        // With est_remaining_floor = 0, the floor check is: time_now > e2e_deadline - 0 = e2e_deadline.
        // Since e2e_deadline is 100ms in the future, this should NOT shed.
        let shed = parent_ctx.est.admission_check(&parent_ctx.base.ctx, 0, 0);
        assert!(!shed, "should admit when plenty of time remains");
    }
}
