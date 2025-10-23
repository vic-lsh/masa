//! Runtime support for configuring and propagating masa-related contexts.

use std::sync::Arc;

use super::MasaHooks;

/// Create the poll hook used to propagate parent context.
pub fn make_child_task_poll_hook<M>(parent_context: Arc<M::ParentContext>) -> tokio::task::PollHook
where
    M: MasaHooks,
{
    let hook_ctx = Arc::into_raw(parent_context) as *const ();
    let child_hook = unsafe {
        tokio::task::PollHook::new(
            hook_ctx,
            Some(hook_impl::on_clone::<M>),
            Some(hook_impl::on_destroy::<M>),
            Some(hook_impl::before_poll::<M>),
            Some(hook_impl::after_poll::<M>),
        )
    };

    child_hook
}

mod hook_impl {
    use std::sync::Arc;

    use crate::masa::MasaHooks;

    // Each child task would clone this request context using this fn.
    pub(crate) fn on_clone<M>(raw_ctx: *const ())
    where
        M: MasaHooks,
    {
        // Bump req-ctx ref-count without losing the original ref-count.
        let c = unsafe { Arc::from_raw(raw_ctx as *const M::ParentContext) };
        let _ = Arc::into_raw(c.clone());
        let _ = Arc::into_raw(c); // don't drop c and lose a refcount.
    }

    // Release context ref-count associated with this child task.
    pub(crate) fn on_destroy<M>(raw_ctx: *const ())
    where
        M: MasaHooks,
    {
        unsafe { Arc::from_raw(raw_ctx as *const M::ParentContext) };
    }

    // Configure child task's thread-local to point to our request context.
    pub(crate) fn before_poll<M>(raw_ctx: *const ())
    where
        M: MasaHooks,
    {
        // SAFETY: the hook holds one ref-count to the request context.
        let ctx = unsafe { &*(raw_ctx as *const M::ParentContext) };
        crate::masa::context::server::set_parent_ctx::<M>(ctx);
    }

    // Remove request context from our thread local to avoid exposing it
    // to another task (and mislead another task to think they have a parent rpc).
    pub(crate) fn after_poll<M>(_raw_ctx: *const ())
    where
        M: MasaHooks,
    {
        crate::masa::context::server::reset_parent_ctx::<M>();
    }
}
