//! Runtime support for configuring and propagating Masa-related contexts.

use std::sync::Arc;

use crate::Hooks;

/// Create the Tokio poll hook used to propagate parent context to child tasks.
pub fn make_child_task_poll_hook<M>(parent_context: Arc<M::ParentContext>) -> tokio::task::PollHook
where
    M: Hooks,
{
    let hook_ctx = Arc::into_raw(parent_context) as *const ();
    unsafe {
        tokio::task::PollHook::new(
            hook_ctx,
            Some(hook_impl::on_clone::<M>),
            Some(hook_impl::on_destroy::<M>),
            Some(hook_impl::before_poll::<M>),
            Some(hook_impl::after_poll::<M>),
        )
    }
}

mod hook_impl {
    use std::sync::Arc;

    use crate::Hooks;

    pub(crate) fn on_clone<M>(raw_ctx: *const ())
    where
        M: Hooks,
    {
        // Bump the request-context ref count without consuming the original count.
        let c = unsafe { Arc::from_raw(raw_ctx as *const M::ParentContext) };
        let _ = Arc::into_raw(c.clone());
        let _ = Arc::into_raw(c);
    }

    pub(crate) fn on_destroy<M>(raw_ctx: *const ())
    where
        M: Hooks,
    {
        unsafe { Arc::from_raw(raw_ctx as *const M::ParentContext) };
    }

    pub(crate) fn before_poll<M>(raw_ctx: *const ())
    where
        M: Hooks,
    {
        // SAFETY: the hook owns one ref count to the request context.
        let ctx = unsafe { &*(raw_ctx as *const M::ParentContext) };
        crate::server::set_parent_ctx::<M>(ctx);
    }

    pub(crate) fn after_poll<M>(_raw_ctx: *const ())
    where
        M: Hooks,
    {
        crate::server::reset_parent_ctx::<M>();
    }
}
