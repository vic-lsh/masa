//! Runtime support for configuring and propagating masa-related contexts.

use std::sync::Arc;

use crate::masa::{ClientStubHooks, RequestHandlerHooks, ServerHooks};

/// Create the poll hook used to propagate parent context.
pub fn make_child_task_poll_hook<S, C, P>(parent_context: Arc<P>) -> tokio::task::PollHook
where
    S: ServerHooks,
    C: ClientStubHooks,
    P: RequestHandlerHooks<C, S>,
{
    let hook_ctx = Arc::into_raw(parent_context) as *const ();
    let child_hook = unsafe {
        tokio::task::PollHook::new(
            hook_ctx,
            Some(hook_impl::on_clone::<S, C, P>),
            Some(hook_impl::on_destroy::<S, C, P>),
            Some(hook_impl::before_poll::<S, C, P>),
            Some(hook_impl::after_poll::<S, C, P>),
        )
    };

    child_hook
}

mod hook_impl {
    use std::sync::Arc;

    use crate::masa::{ClientStubHooks, RequestHandlerHooks, ServerHooks};

    // Each child task would clone this request context using this fn.
    pub(crate) fn on_clone<S, C, P>(raw_ctx: *const ())
    where
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    {
        // Bump req-ctx ref-count without losing the original ref-count.
        let c = unsafe { Arc::from_raw(raw_ctx as *const P) };
        let _ = Arc::into_raw(c.clone());
        let _ = Arc::into_raw(c); // don't drop c and lose a refcount.
    }

    // Release context ref-count associated with this child task.
    pub(crate) fn on_destroy<S, C, P>(raw_ctx: *const ())
    where
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    {
        unsafe { Arc::from_raw(raw_ctx as *const P) };
    }

    // Configure child task's thread-local to point to our request context.
    pub(crate) fn before_poll<S, C, P>(raw_ctx: *const ())
    where
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    {
        // SAFETY: the hook holds one ref-count to the request context.
        let ctx = unsafe { &*(raw_ctx as *const P) };
        crate::masa::context::server::set_parent_ctx::<S, C, P>(ctx);
    }

    // Remove request context from our thread local to avoid exposing it
    // to another task (and mislead another task to think they have a parent rpc).
    pub(crate) fn after_poll<S, C, P>(_raw_ctx: *const ())
    where
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    {
        crate::masa::context::server::reset_parent_ctx::<S, C, P>();
    }
}
