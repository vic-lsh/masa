use crate::{
    body::BoxBody, masa::context::read_context, Code, GrpcMethod, Request, Response, Status,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    task::Poll,
    time::Instant,
};

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::{estimate_method_latency, track_method_latency};
use masa::{time_now, Context, LatencyDistribution, MethodId, EARLY_RETURN};

#[derive(Debug)]
/// This policy computes the deadline d of a child request as  
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlineDirect;

impl MasaHooks for LocalDeadlineDirect {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext {
    // for every method on this server, tracks the remaining duration of the method after an outgoing request has finished
    child_distributions: RwLock<HashMap<String, LatencyDistribution>>,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            child_distributions: RwLock::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    method: GrpcMethod,
    ctx: Context,
    server: Arc<ServerContext>,

    will_early_return: AtomicBool,
    child_end_times: Mutex<Vec<(MethodId, Instant)>>,
}

impl ParentContext {
    #[inline]
    fn check_early_return(&self) -> bool {
        if EARLY_RETURN {
            self.check_early_return_impl()
        } else {
            false
        }
    }

    fn check_early_return_impl(&self) -> bool {
        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= self.ctx.deadline();

        if should_early_return {
            if self
                .will_early_return
                .compare_exchange_weak(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {}
        }

        should_early_return
    }

    #[inline]
    fn issue_early_return(&self) -> Status {
        Status::new(Code::DeadlineExceeded, format!("/EarlyReturn"))
    }
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method,
            ctx: read_context(req),
            server: server_ctx,
            will_early_return: AtomicBool::new(false),
            child_end_times: Mutex::new(Vec::new()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.check_early_return() {
            return Err(Err(self.issue_early_return()));
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.check_early_return() {
                return Err(Err(self.issue_early_return()));
            }
        }

        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        // NOTE: we need to include the parent method in the key, because the duration until the
        // end of the parent request after this child request completes will vary for different
        // parent methods (i.e. endpoints on this server)
        let estimate_remaining = estimate_method_latency(
            &self.server.child_distributions,
            format!("{}/{}", self.method.id(), child_method.id()),
        )
        .unwrap_or(0);

        // let estimate_remaining = 10 as u64;
        // NOTE(vic): could we have passed the deadline at this point?
        let deadline = self.ctx.deadline() - estimate_remaining;

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.start_at(),
            deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        self.child_end_times
            .lock()
            .unwrap()
            .push((child_method.id(), Instant::now()));

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let parent_end = Instant::now();
        // track remaining time after each child
        for (child_method, child_end) in self.child_end_times.lock().unwrap().iter() {
            // TODO: The LatencyDistribution instances will regularly sort their data. Should this
            // work be done asynchronously?
            track_method_latency(
                &self.server.child_distributions,
                format!("{}/{}", self.method.id(), child_method),
                parent_end.duration_since(*child_end).as_micros() as u64,
            );
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
