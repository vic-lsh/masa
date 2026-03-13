use crate::{masa::context::read_context, CowGrpcMethod, GrpcMethod, Request, Status};
use std::sync::Arc;
use std::task::Poll;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::common::{EarlyReturnHandler, QueueLatencyTracker};
use super::rajomon::RajomonHandler;
use super::{resolve_method_name_from_http, resolve_method_name_from_request, MasaRequestExt};
use crate::Response;
use masa_core::{Context, ContextBuilder};

#[cfg(feature = "adctl")]
use super::adctl_hooks::{
    is_early_return_response, AdctlChildState, AdctlRequestState, AdctlServerState,
};
#[cfg(feature = "adctl")]
use super::estimator::DefaultLatencyEstimator;
#[cfg(feature = "adctl")]
use crate::masa::MethodRegistry;

#[derive(Debug)]
/// FIFO policy with optional early return support.
/// Requests are served in first-in-first-out order.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct Fifo;

impl MasaHooks for Fifo {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ServerContext {
    #[cfg(feature = "adctl")]
    adctl: Arc<AdctlServerState<DefaultLatencyEstimator>>,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            #[cfg(feature = "adctl")]
            adctl: Arc::new(AdctlServerState::new()),
        }
    }
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    ctx: Context,
    q_lat_tracker: QueueLatencyTracker,
    early_return: EarlyReturnHandler,
    #[cfg(feature = "adctl")]
    adctl: AdctlRequestState<DefaultLatencyEstimator>,
    rajomon: RajomonHandler,
}

/// Resolve the method name from Request metadata, checking for override header.

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        let mut ctx = read_context(req);
        let resolved_method = resolve_method_name_from_http(method, req);
        let mut rajomon = RajomonHandler::new(resolved_method.clone());
        rajomon.check_inbound(&mut ctx);

        #[cfg(feature = "adctl")]
        let resolved_method_id = MethodRegistry::global()
            .get_or_register_method(resolved_method.service(), resolved_method.method());

        Self {
            ctx,
            q_lat_tracker: QueueLatencyTracker::new(),
            early_return: EarlyReturnHandler::new(resolved_method),
            #[cfg(feature = "adctl")]
            adctl: AdctlRequestState::new(resolved_method_id, _server_ctx.adctl.clone()),
            rajomon,
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.rajomon.should_drop() {
            return Err(Err(self.rajomon.issue_error(None)));
        }

        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }

        self.rajomon.track_queue_delay();
        self.q_lat_tracker.track_poll();
        #[cfg(feature = "adctl")]
        self.adctl.start_compute_tracking();
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.rajomon.should_drop() {
            return Err(self.rajomon.issue_error(None));
        }

        if self.early_return.check(&self.ctx) {
            return Err(self.early_return.issue_error());
        }

        let child_method_name = resolve_method_name_from_request(child_method, request);
        self.rajomon.check_outbound(&child_method_name, &self.ctx)?;

        child_ctx.set_method_name(child_method_name.clone());

        #[cfg(feature = "adctl")]
        if self
            .adctl
            .prepare_before_child_rpc(&self.ctx, &child_method_name, &mut child_ctx.adctl)
            .is_err()
        {
            return Err(self.early_return.issue_error());
        }

        let deadline = self.ctx.deadline();
        let prio_hint = self.ctx.prio_hint();

        #[allow(unused_mut)]
        let mut builder = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(prio_hint);

        #[cfg(feature = "adctl")]
        {
            builder = builder.hop_count(self.ctx.hop_count().saturating_add(1));
        }

        let child_recv_ctx = builder.build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        self.q_lat_tracker.track_child_response(response);

        #[cfg(feature = "adctl")]
        {
            child_ctx.adctl.finalize(response);
            self.adctl
                .after_child_rpc(&self.ctx, response, &child_ctx.adctl);
        }

        if let Some(child) = child_ctx.child_method_name {
            if let Ok(resp) = response {
                self.rajomon
                    .update_cache_from_response(&child, resp.metadata());
            } else if let Err(status) = response {
                self.rajomon
                    .update_cache_from_response(&child, status.metadata());
            }
            self.early_return.set_last_child(child);
        }
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(feature = "adctl")]
        self.adctl.stop_compute_tracking();

        match poll {
            Poll::Pending => {
                if self.rajomon.should_drop() {
                    return Err(Err(self.rajomon.issue_error(None)));
                }
                if self.early_return.check(&self.ctx) {
                    return Err(Err(self.early_return.issue_error()));
                }
            }
            Poll::Ready(_) => {}
        };

        Ok(())
    }

    // expect frontend method, all other method are going send back their latency trace
    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        #[cfg(feature = "adctl")]
        {
            if !is_early_return_response(result) {
                self.adctl.track_latencies();
            }
            self.adctl.inject_response_meta(&self.ctx, result);
        }

        self.rajomon.finalize_queue_delay();
        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
        self.rajomon.inject_price_to_response(result);
    }
}

#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct ChildContext {
    pub child_method_name: Option<CowGrpcMethod>,
    #[cfg(feature = "adctl")]
    adctl: AdctlChildState<DefaultLatencyEstimator>,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            #[cfg(feature = "adctl")]
            adctl: AdctlChildState::new(),
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
    crate::generate_early_return_test!(ParentContext, ServerContext, ChildContext);
}
