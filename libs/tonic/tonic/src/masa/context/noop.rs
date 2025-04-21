use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tonic_masa::{time_now, Context, FIFO_EARLY};

use crate::{Code, GrpcMethod, Request, Status};

use super::{read_context, ClientHooks, ParentHooks, PrioritySelector, ServerHooks};

#[derive(Debug)]
pub struct NoopPrioritySelector;

impl PrioritySelector for NoopPrioritySelector {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

/// A noop implementation of `ParentHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {
    will_early_return: AtomicBool,
    deadline: u64,
}

/// A simple implementation of `ClientHooks`.
#[derive(Debug, Clone)]
pub struct ChildContext {}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {}

impl ParentContext {
    // TODO: DRY (same code in simple.rs)
    #[inline]
    fn check_early_return(&self) -> bool {
        if FIFO_EARLY {
            if self.will_early_return.load(Ordering::Relaxed) {
                return true;
            }

            let now = time_now();
            let should_early_return = now >= self.deadline;

            if should_early_return {
                // `check_early_return` may be invoked at multiple lifecycle hooks.
                //
                // this will only be read/written on one thread, so we can use the
                // weakest ordering guarantees.
                // it is an atomic because the ParentContext type needs to be Sync:
                // see the docs for ParentHooks for why.
                let _ = self.will_early_return.compare_exchange_weak(
                    false,
                    true,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            return should_early_return;
        } else {
            return false;
        }
    }

    #[inline]
    fn issue_early_return(&self, method: GrpcMethod) -> Status {
        Status::new(
            Code::DeadlineExceeded,
            format!("/EarlyReturn{}", method.id()),
        )
    }
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            will_early_return: AtomicBool::new(false),
            deadline: read_context(req).deadline(),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return(method));
        }

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _response: &mut Result<crate::Response<T>, crate::Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return(method));
        }

        Ok(())
    }
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}
