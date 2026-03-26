use crate::{CowGrpcMethod, GrpcMethod, Request, Status};
use std::sync::Arc;
use std::task::Poll;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::base::BaseHookState;
use super::{resolve_method_name_from_request, MasaRequestExt};
use crate::Response;
use masa_core::ContextBuilder;

#[cfg(feature = "ac_est")]
use super::est::estimator::DefaultLatencyEstimator;
#[cfg(feature = "ac_est")]
use super::est::state::{is_early_return_response, EstChildState, EstRequestState, EstServerState};
#[cfg(feature = "ac_est")]
use crate::masa::MethodRegistry;

#[derive(Debug)]
/// Standard Masa hooks implementation shared by FIFO, priority, and tailclipper policies.
/// The actual scheduling differences are handled by the tokio runtime, not these hooks.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct StandardHooks;

impl MasaHooks for StandardHooks {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ServerContext {
    #[cfg(feature = "ac_est")]
    est: Arc<EstServerState<DefaultLatencyEstimator>>,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            #[cfg(feature = "ac_est")]
            est: Arc::new(EstServerState::new()),
        }
    }
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    base: BaseHookState,
    #[cfg(feature = "ac_est")]
    est: EstRequestState<DefaultLatencyEstimator>,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        let base = BaseHookState::new(method, req);

        #[cfg(feature = "ac_est")]
        let resolved_method_id = MethodRegistry::global().get_or_register_method(
            base.resolved_method.service(),
            base.resolved_method.method(),
        );

        Self {
            base,
            #[cfg(feature = "ac_est")]
            est: EstRequestState::new(resolved_method_id, _server_ctx.est.clone()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.base.check_guards()?;
        self.base.track_poll();
        #[cfg(feature = "ac_est")]
        self.est.start_compute_tracking();
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
            .rajomon
            .check_outbound(&child_method_name, &self.base.ctx)?;

        child_ctx.set_method_name(child_method_name.clone());

        #[cfg(feature = "ac_est")]
        if self
            .est
            .prepare_before_child_rpc(&self.base.ctx, &child_method_name, &mut child_ctx.est)
            .is_err()
        {
            return Err(self.base.slo_abort.issue_error());
        }

        let deadline = self.base.ctx.deadline();
        let prio_hint = self.base.ctx.prio_hint();

        #[allow(unused_mut)]
        let mut builder = ContextBuilder::from(&self.base.ctx)
            .deadline(deadline)
            .prio_hint(prio_hint)
            .tokens(self.base.rajomon.remaining_tokens());

        #[cfg(feature = "ac_est")]
        {
            builder = builder.hop_count(self.base.ctx.hop_count().saturating_add(1));
        }

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

        #[cfg(feature = "ac_est")]
        {
            child_ctx.est.finalize(response);
            self.est
                .after_child_rpc(&self.base.ctx, response, &child_ctx.est);
        }

        if let Some(child) = child_ctx.child_method_name {
            self.base.update_after_child(&child, response);
        }
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(feature = "ac_est")]
        self.est.stop_compute_tracking();

        self.base.check_pending_guards(poll)
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        #[cfg(feature = "ac_est")]
        {
            if !is_early_return_response(result) {
                self.est.track_latencies();
            }
            self.est.inject_response_meta(&self.base.ctx, result);
        }

        self.base.finalize(result);
    }
}

#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct ChildContext {
    pub child_method_name: Option<CowGrpcMethod>,
    #[cfg(feature = "ac_est")]
    est: EstChildState<DefaultLatencyEstimator>,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            #[cfg(feature = "ac_est")]
            est: EstChildState::new(),
        }
    }
}

impl ChildContext {
    pub(super) fn set_method_name(&mut self, name: CowGrpcMethod) {
        self.child_method_name = Some(name);
    }
}

#[cfg(test)]
mod tests {
    crate::generate_slo_abort_test!(ParentContext, ServerContext, ChildContext);
}
