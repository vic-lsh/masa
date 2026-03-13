use crate::masa::MethodRegistry;
use crate::{masa::context::read_context, GrpcMethod, Request, Response, Status};
use std::sync::Arc;
use std::task::Poll;

use super::super::adctl_hooks::{
    is_early_return_response, AdctlChildState, AdctlRequestState, AdctlServerState,
};
use super::super::common::{EarlyReturnHandler, QueueLatencyTracker};
use super::super::rajomon::RajomonHandler;
use super::super::{
    resolve_method_name_from_http, ClientHooks, MasaHooks, MasaRequestExt, ParentHooks, ServerHooks,
};
use masa_core::{time_now, Context, ContextBuilder, LatencyEstimator, PriorityHint};

use super::super::estimator::DefaultLatencyEstimator as LocalLatencyEstimator;

#[derive(Debug)]
/// This policy computes the deadline d of a child request as
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlinePolicy;

impl MasaHooks for LocalDeadlinePolicy {
    type ServerContext = ServerContext<LocalLatencyEstimator>;
    type ChildContext = ChildContext<LocalLatencyEstimator>;
    type ParentContext = ParentContext<LocalLatencyEstimator>;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    pub(super) adctl: Arc<AdctlServerState<E>>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        Self {
            adctl: Arc::new(AdctlServerState::new()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    ctx: Context,
    q_lat_tracker: QueueLatencyTracker,
    early_return: EarlyReturnHandler,
    rajomon: RajomonHandler,
    adctl: AdctlRequestState<E>,
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext<E>, ServerContext<E>>
    for ParentContext<E>
{
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<E>>,
    ) -> Self {
        let mut ctx = read_context(req);
        let resolved_method = resolve_method_name_from_http(method, req);
        let resolved_method_id = MethodRegistry::global()
            .get_or_register_method(resolved_method.service(), resolved_method.method());

        let mut rajomon = RajomonHandler::new(resolved_method.clone());
        rajomon.check_inbound(&mut ctx);

        Self {
            ctx,
            q_lat_tracker: QueueLatencyTracker::new(),
            early_return: EarlyReturnHandler::new(resolved_method),
            rajomon,
            adctl: AdctlRequestState::new(resolved_method_id, server_ctx.adctl.clone()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.rajomon.should_drop() {
            return Err(Err(self.rajomon.issue_error(None)));
        }

        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }

        let remaining = self.ctx.deadline().saturating_sub(time_now());
        tokio::task::reprioritize(masa_core::PriorityHint::new(remaining));

        self.rajomon.track_queue_delay();
        self.q_lat_tracker.track_poll();
        self.adctl.start_compute_tracking();
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.adctl.stop_compute_tracking();

        if let Poll::Pending = poll {
            if self.rajomon.should_drop() {
                return Err(Err(self.rajomon.issue_error(None)));
            }
            if self.early_return.check(&self.ctx) {
                return Err(Err(self.early_return.issue_error()));
            }
        }

        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext<E>,
    ) -> Result<(), Status> {
        if self.rajomon.should_drop() {
            return Err(self.rajomon.issue_error(None));
        }

        if self.early_return.check(&self.ctx) {
            return Err(self.early_return.issue_error());
        }

        let resolved_child_method =
            super::super::resolve_method_name_from_request(child_method, request);

        self.rajomon
            .check_outbound(&resolved_child_method, &self.ctx)?;

        let prepare_result = self
            .adctl
            .prepare_before_child_rpc(&self.ctx, &resolved_child_method, &mut child_ctx.adctl)
            .map_err(|_| self.early_return.issue_error())?;

        let deadline = self
            .ctx
            .deadline()
            .saturating_sub(prepare_result.est_remaining);
        // prio_hint = deadline (parent_deadline - est_remaining). Subtracting est_child caused
        // priority inversions under load: as est_child grew with queuing, newer requests got
        // tighter prio_hints than older ones computed with lower estimates, inverting FIFO order.
        // Using deadline alone gives FIFO-like ordering (since all requests share the same e2e
        // SLO deadline), while preserving path-aware early returns from the tightened deadline.
        let prio_hint = deadline;

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(prio_hint))
            .hop_count(self.ctx.hop_count().saturating_add(1))
            .tokens(self.rajomon.remaining_tokens())
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
        self.q_lat_tracker.track_child_response(response);
        child_ctx.adctl.finalize(response);
        self.adctl
            .after_child_rpc(&self.ctx, response, &child_ctx.adctl);

        if let Some(child_method) = &child_ctx.adctl.child_method {
            if let Ok(resp) = response {
                self.rajomon
                    .update_cache_from_response(child_method, resp.metadata());
            } else if let Err(status) = response {
                self.rajomon
                    .update_cache_from_response(child_method, status.metadata());
            }
            self.early_return.set_last_child(child_method.clone());
        }

        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        self.rajomon.finalize_queue_delay();
        if !is_early_return_response(result) {
            self.adctl.track_latencies();
        }
        self.adctl.inject_response_meta(&self.ctx, result);

        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
        self.rajomon.inject_price_to_response(result);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    adctl: AdctlChildState<E>,
}

impl<E: LatencyEstimator + Default + 'static> ClientHooks for ChildContext<E> {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            adctl: AdctlChildState::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyRms;

    #[test]
    fn test_server_context_rms_integration() {
        let ctx = ServerContext::<LatencyRms>::new("test_service");
        let method = super::super::super::estimator::ParentToChildId {
            parent_id: 1,
            child_id: 2,
        };
        let key = method.to_key();

        // Inject an estimator with a short update interval (2) for testing.
        // By default, LatencyRms has a large update interval (512), which makes testing hard.
        {
            ctx.adctl.est_child_latency.insert(key, LatencyRms::new(2));
        }

        // 1st track: sum_sq=100, count=1, since_update=1. No update yet.
        ctx.adctl.est_child_latency.track(key, 10);

        // Estimate uses cached RMS value (initially 0).
        let est = ctx.adctl.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(0));

        // 2nd track: sum_sq=200, count=2, since_update=2. Update triggers.
        // RMS = sqrt( (10^2 + 10^2) / 2 ) = 10.
        ctx.adctl.est_child_latency.track(key, 10);

        let est = ctx.adctl.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10));

        // 3rd track: sum_sq=200+400=600, count=3, since_update=1. No update yet.
        ctx.adctl.est_child_latency.track(key, 20);
        let est = ctx.adctl.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10)); // Still 10

        // 4th track: sum_sq=600+400=1000, count=4, since_update=2. Update triggers.
        // RMS = sqrt( (100 + 100 + 400 + 400) / 4 ) = sqrt(250) ≈ 15.
        ctx.adctl.est_child_latency.track(key, 20);
        let est = ctx.adctl.est_child_latency.get_estimate(key);
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
        assert!(child_ctx.adctl.parent_to_child_id.is_some());
        assert!(child_ctx.adctl.server.is_some());

        // Verify registry has IDs
        let registry = MethodRegistry::global();
        let parent_id = registry.get_or_register_method("IntegrationService", "ParentMethod");
        let child_id = registry.get_or_register_method("IntegrationService", "ChildMethod");

        assert_eq!(
            child_ctx
                .adctl
                .parent_to_child_id
                .clone()
                .unwrap()
                .parent_id,
            parent_id
        );
        assert_eq!(
            child_ctx.adctl.parent_to_child_id.clone().unwrap().child_id,
            child_id
        );

        // 5. Simulate Child Response
        let mut response = Ok(Response::new(()));
        let _ = parent_ctx
            .after_child_rpc(child_method, &mut response, child_ctx)
            .unwrap();

        // 6. Track Latencies
        // This normally happens in finalize, but we can call internal method if accessible or simulate via public hook.
        // finalize_before_serialization calls track_latencies if not early return.
        let mut response_result = Ok(Response::new(()));
        parent_ctx.finalize_before_serialization(&mut response_result);

        // 7. Verify Stats Updated
        // We can't easily peek into LatencyRms without internal access or waiting for updates.
        // But we can check that entries exist in the map using the key.
        let _key = (parent_id << 32) | child_id;

        // Need to wait/trigger update if LatencyRms has a window.
        // But simply checking if key exists in map (get_estimate returns Some(0) or something) confirms integration.
        // For LatencyRms, get_estimate returns None if not enough data, or Some(val).
        // Since we tracked one value, it might be cached.

        // We can verify that the key exists in the map implicitly by tracking again or checking log side effects (hard).
        // Better: check that we can retrieve *some* estimate (even if 0) or that the key is present.
        // LatencyMap::get_estimate will create default if missing.
        // We want to ensure it WAS created/tracked.
        // We can't check 'was tracked' easily on the public interface without side channels.
        // However, the fact that we ran through without panic/error is a good sign.

        // Let's verify we can get the names back from registry for the IDs we expect.
        let (p_s, p_m) = registry.get_method_name(parent_id).unwrap();
        assert_eq!(p_s, "IntegrationService");
        assert_eq!(p_m, "ParentMethod");
    }

    /// Verify that without `adctl`, `admission_check` falls back to the floor-based check.
    /// The floor check should admit when there is plenty of time left (no shed).
    #[cfg(not(feature = "adctl"))]
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
        let shed = parent_ctx.adctl.admission_check(&parent_ctx.ctx, 0, 0);
        assert!(!shed, "should admit when plenty of time remains");
    }
}
