// Masa hooks implementation.
//
// `PolicyHooks` is the single concrete `Hooks` implementation used by all
// scheduling policies (sched_fifo, sched_slo, sched_tailclipper, sched_oracle, sched_pred).
// The actual scheduling differences are handled by the tokio runtime and,
// when enabled, the active layers (estimation, oracle, and/or admission).
//
// Layers are called in field order:
//   e2e_deadline_guard → estimation → oracle → admission → queue_latency.
// The first `Err` short-circuits.

use std::sync::Arc;
use std::task::Poll;

use crate::context_ext::{read_context, MasaRequestExt, MasaResponseExt, MasaStatusExt};
use crate::layer::{
    AdmissionLayer, ChildRpcContext, E2eDeadlineGuardLayer, EstimationLayer, Layer, LayerChild,
    LayerServer, OracleLayer, QueueLatencyLayer,
};
use masa_core::{Context, ContextBuilder};
use tonic_core::masa_ext::resolve_method_name_from_http;
use tonic_core::masa_ext::resolve_method_name_from_request;
use tonic_core::masa_ext::{ClientHooks, Hooks, ParentHooks, ServerHooks};
use tonic_core::{CowGrpcMethod, GrpcMethod, Request, Response, Status};

/// Invoke `$body` for each layer in field order
/// (e2e_deadline_guard → estimation → oracle → admission → queue_latency).
/// The first `Err` short-circuits via `?` if the body uses it.
///
/// Forms:
///   for_each_layer!(self, |o| body)                      — parent layer only
///   for_each_layer!(self, mut child, |o, c| body)        — with mutable child field
///   for_each_layer!(self, child, |o, c| body)            — with immutable child field
macro_rules! for_each_layer {
    ($self:ident, |$o:ident| $body:expr) => {{
        {
            let $o = &$self.e2e_deadline_guard;
            $body
        }
        {
            let $o = &$self.estimation;
            $body
        }
        {
            let $o = &$self.oracle;
            $body
        }
        {
            let $o = &$self.admission;
            $body
        }
        {
            let $o = &$self.queue_latency;
            $body
        }
    }};
    ($self:ident, mut $child:ident, |$o:ident, $c:ident| $body:expr) => {{
        {
            let $o = &$self.e2e_deadline_guard;
            let $c = &mut $child.e2e_deadline_guard;
            $body
        }
        {
            let $o = &$self.estimation;
            let $c = &mut $child.estimation;
            $body
        }
        {
            let $o = &$self.oracle;
            let $c = &mut $child.oracle;
            $body
        }
        {
            let $o = &$self.admission;
            let $c = &mut $child.admission;
            $body
        }
        {
            let $o = &$self.queue_latency;
            let $c = &mut $child.queue_latency;
            $body
        }
    }};
    ($self:ident, $child:ident, |$o:ident, $c:ident| $body:expr) => {{
        {
            let $o = &$self.e2e_deadline_guard;
            let $c = &$child.e2e_deadline_guard;
            $body
        }
        {
            let $o = &$self.estimation;
            let $c = &$child.estimation;
            $body
        }
        {
            let $o = &$self.oracle;
            let $c = &$child.oracle;
            $body
        }
        {
            let $o = &$self.admission;
            let $c = &$child.admission;
            $body
        }
        {
            let $o = &$self.queue_latency;
            let $c = &$child.queue_latency;
            $body
        }
    }};
}

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
    e2e_deadline_guard: <E2eDeadlineGuardLayer as Layer>::Server,
    estimation: <EstimationLayer as Layer>::Server,
    oracle: <OracleLayer as Layer>::Server,
    admission: <AdmissionLayer as Layer>::Server,
    queue_latency: <QueueLatencyLayer as Layer>::Server,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            e2e_deadline_guard: <<E2eDeadlineGuardLayer as Layer>::Server as LayerServer>::new(),
            estimation: <<EstimationLayer as Layer>::Server as LayerServer>::new(),
            oracle: <<OracleLayer as Layer>::Server as LayerServer>::new(),
            admission: <<AdmissionLayer as Layer>::Server as LayerServer>::new(),
            queue_latency: <<QueueLatencyLayer as Layer>::Server as LayerServer>::new(),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {
    ctx: Context,
    resolved_method: CowGrpcMethod,
    pub(crate) e2e_deadline_guard: E2eDeadlineGuardLayer,
    pub(crate) estimation: EstimationLayer,
    pub(crate) oracle: OracleLayer,
    pub(crate) admission: AdmissionLayer,
    pub(crate) queue_latency: QueueLatencyLayer,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        let mut ctx = read_context(req);
        let resolved_method = resolve_method_name_from_http(method, req);

        let e2e_deadline_guard =
            E2eDeadlineGuardLayer::new(&resolved_method, &server_ctx.e2e_deadline_guard, &mut ctx);
        let estimation = EstimationLayer::new(&resolved_method, &server_ctx.estimation, &mut ctx);
        let oracle = OracleLayer::new(&resolved_method, &server_ctx.oracle, &mut ctx);
        let admission = AdmissionLayer::new(&resolved_method, &server_ctx.admission, &mut ctx);
        let queue_latency =
            QueueLatencyLayer::new(&resolved_method, &server_ctx.queue_latency, &mut ctx);

        Self {
            ctx,
            resolved_method,
            e2e_deadline_guard,
            estimation,
            oracle,
            admission,
            queue_latency,
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        for_each_layer!(self, |o| o.before_poll(&self.ctx)?);
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        let child_method_name = resolve_method_name_from_request(child_method, request);
        child_ctx.set_method_name(child_method_name.clone());

        let mut child_rpc = ChildRpcContext::from_parent(&self.ctx);

        for_each_layer!(self, mut child_ctx, |o, c| o.before_child_rpc(
            &self.ctx,
            &child_method_name,
            c,
            request,
            &mut child_rpc
        )?);

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(child_rpc.deadline)
            .prio_hint(child_rpc.prio_hint)
            .hop_count(child_rpc.hop_count)
            .tokens(child_rpc.tokens)
            .build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Some(child_method) = child_ctx.child_method_name.as_ref() {
            for_each_layer!(self, child_ctx, |o, c| o.after_child_rpc(
                &self.ctx,
                child_method,
                response,
                c
            )?);
        }
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        for_each_layer!(self, |o| o.after_poll(&self.ctx, poll)?);
        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        let mut ctx = self.ctx.clone();
        for_each_layer!(self, |o| o.finalize(&mut ctx, result));
        match result {
            Ok(resp) => resp.set_masa_context(&ctx),
            Err(status) => status.set_masa_context(&ctx),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {
    pub child_method_name: Option<CowGrpcMethod>,
    e2e_deadline_guard: <E2eDeadlineGuardLayer as Layer>::Child,
    pub(crate) estimation: <EstimationLayer as Layer>::Child,
    pub(crate) oracle: <OracleLayer as Layer>::Child,
    pub(crate) admission: <AdmissionLayer as Layer>::Child,
    queue_latency: <QueueLatencyLayer as Layer>::Child,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            e2e_deadline_guard: <<E2eDeadlineGuardLayer as Layer>::Child as LayerChild>::new(),
            estimation: <<EstimationLayer as Layer>::Child as LayerChild>::new(),
            oracle: <<OracleLayer as Layer>::Child as LayerChild>::new(),
            admission: <<AdmissionLayer as Layer>::Child as LayerChild>::new(),
            queue_latency: <<QueueLatencyLayer as Layer>::Child as LayerChild>::new(),
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
    crate::generate_abort_slo_test!(ParentContext, ServerContext, ChildContext);

    #[cfg(feature = "estimator")]
    mod est_tests {
        use super::super::{ChildContext, ParentContext, ServerContext};
        use crate::context_ext::MASA_CONTEXT_HEADER;
        use crate::layer::est::latency_map::ParentToChildKey;
        use crate::layer::est::state::LatencyEstimators;
        use crate::MethodRegistry;
        use masa_core::{ContextBuilder, LatencyRms};
        use std::sync::Arc;
        use tonic_core::masa_ext::resolve_method_name_from_http;
        use tonic_core::masa_ext::{ClientHooks, ParentHooks, ServerHooks};
        use tonic_core::{CowGrpcMethod, GrpcMethod, Request, Response};

        #[test]
        fn test_server_context_rms_integration() {
            let est = LatencyEstimators::<LatencyRms>::new();
            let registry = MethodRegistry::global();
            let parent_mid =
                registry.get_or_register(CowGrpcMethod::new("TestIntegration", "Parent"));
            let child_mid =
                registry.get_or_register(CowGrpcMethod::new("TestIntegration", "Child"));
            let root_mid = registry.get_or_register(CowGrpcMethod::new("TestIntegration", "Root"));
            let key = ParentToChildKey::root_rpc_method(root_mid)
                .parent_rpc_method(parent_mid)
                .child_rpc_method(child_mid);

            {
                est.child_wallclock_map().insert(key, LatencyRms::new(2));
            }

            est.track_child_wallclock(key, 10);
            let val = est.est_child_wallclock(key);
            assert_eq!(val, Some(0));

            est.track_child_wallclock(key, 10);
            let val = est.est_child_wallclock(key);
            assert_eq!(val, Some(10));

            est.track_child_wallclock(key, 20);
            let val = est.est_child_wallclock(key);
            assert_eq!(val, Some(10));

            est.track_child_wallclock(key, 20);
            let val = est.est_child_wallclock(key);
            assert_eq!(val, Some(15));
        }

        #[test]
        fn test_resolve_method_name_from_http_with_overrides() {
            use http::HeaderValue;
            use tonic_core::masa_ext::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};

            let method = GrpcMethod::new("TestService", "TestMethod");
            let mut req = http::Request::new(());

            let resolved = resolve_method_name_from_http(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "TestMethod");

            req.headers_mut().insert(
                METHOD_NAME_OVERRIDE_HEADER,
                HeaderValue::from_static("OverriddenMethod"),
            );
            let resolved = resolve_method_name_from_http(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "OverriddenMethod");

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

            let resolved = resolve_method_name_from_request(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "TestMethod");

            req.metadata_mut().insert(
                METHOD_NAME_OVERRIDE_HEADER,
                MetadataValue::from_static("OverriddenMethod"),
            );
            let resolved = resolve_method_name_from_request(method, &req);
            assert_eq!(resolved.service(), "TestService");
            assert_eq!(resolved.method(), "OverriddenMethod");

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
            let server_ctx = Arc::new(ServerContext::new("IntegrationService"));

            let method = GrpcMethod::new("IntegrationService", "ParentMethod");
            let now = masa_core::time_now();
            let slo_us = 100_000u64;
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

            let parent_ctx = ParentContext::begin(method, &req, server_ctx.clone());

            let child_method = GrpcMethod::new("IntegrationService", "ChildMethod");
            let mut child_req = Request::new(());
            let mut child_ctx = ChildContext::new(child_method, &child_req);

            let _ = parent_ctx
                .before_child_rpc(child_method, &mut child_req, &mut child_ctx)
                .unwrap();

            // Verify child tracker was initialized by the estimation layer
            let child_tracker = child_ctx
                .estimation
                .child_tracker
                .as_ref()
                .expect("child_tracker should be initialized after before_child_rpc");

            let registry = MethodRegistry::global();
            let parent_id =
                registry.get_or_register(CowGrpcMethod::new("IntegrationService", "ParentMethod"));
            let child_id =
                registry.get_or_register(CowGrpcMethod::new("IntegrationService", "ChildMethod"));

            let key = child_tracker.key;
            assert_eq!(key.parent(), parent_id);
            assert_eq!(key.child(), child_id);

            let mut response = Ok(Response::new(()));
            let _ = parent_ctx
                .after_child_rpc(child_method, &mut response, child_ctx)
                .unwrap();

            let mut response_result = Ok(Response::new(()));
            parent_ctx.finalize_before_serialization(&mut response_result);

            let parent_method = registry.get_method_name(parent_id).unwrap();
            assert_eq!(parent_method.service(), "IntegrationService");
            assert_eq!(parent_method.method(), "ParentMethod");
        }
    }
}
