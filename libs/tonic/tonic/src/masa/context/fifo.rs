use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use crate::{Code, Response};
use masa::{time_now, Context, EARLY_RETURN};

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
pub struct ServerContext {}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    ctx: Context,
    will_early_return: AtomicBool,
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
            // `check_early_return` may be invoked at multiple lifecycle hooks.
            //
            // this will only be read/written on one thread, so we can use the
            // weakest ordering guarantees.
            // it is an atomic because the ParentContext type needs to be Sync:
            // see the docs for ParentHooks for why.
            if self
                .will_early_return
                .compare_exchange_weak(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                // error!("Request going to early return");
            }
        }

        return should_early_return;
    }

    #[inline]
    fn issue_early_return(&self) -> Status {
        Status::new(Code::DeadlineExceeded, format!("/EarlyReturn"))
    }
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            ctx: read_context(req),
            will_early_return: AtomicBool::new(false),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.check_early_return() {
            return Err(Err(self.issue_early_return()));
        }
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        let deadline = self.ctx.deadline();

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.start_at(),
            deadline,
            deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        match poll {
            Poll::Pending => {
                if self.check_early_return() {
                    return Err(Err(self.issue_early_return()));
                }
            }
            Poll::Ready(_) => {}
        };

        Ok(())
    }
}

#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
