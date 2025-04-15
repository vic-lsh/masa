//! Runtime support for configuring and propagating masa-related contexts.
//!
//! Currently support is limited to async-task + async-executor.

/// Support masa context in `async_executor`.
pub mod async_executor {
    use std::sync::Arc;

    use crate::masa::PrioritySelector;

    /// Create the poll hook used to propagate parent context in async-executor.
    pub fn make_child_task_poll_hook<P>(
        parent_context: Arc<P::ParentContext>,
    ) -> crate::async_task::RawPollHook
    where
        P: PrioritySelector,
    {
        let hook_ctx = Arc::into_raw(parent_context) as *const ();
        let child_hook = unsafe {
            crate::async_task::RawPollHook::new(
                hook_ctx,
                Some(hook_impl::on_clone::<P>),
                Some(hook_impl::on_destroy::<P>),
                Some(hook_impl::before_poll::<P>),
                Some(hook_impl::after_poll::<P>),
            )
        };

        child_hook
    }

    mod hook_impl {
        use std::sync::Arc;

        use crate::masa::PrioritySelector;

        // Each child task would clone this request context using this fn.
        pub(crate) fn on_clone<P>(raw_ctx: *const ())
        where
            P: PrioritySelector,
        {
            // Bump req-ctx ref-count without losing the original ref-count.
            let c = unsafe { Arc::from_raw(raw_ctx as *const P::ParentContext) };
            let _ = Arc::into_raw(c.clone());
            let _ = Arc::into_raw(c); // don't drop c and lose a refcount.
        }

        // Release context ref-count associated with this child task.
        pub(crate) fn on_destroy<P>(raw_ctx: *const ())
        where
            P: PrioritySelector,
        {
            unsafe { Arc::from_raw(raw_ctx as *const P::ParentContext) };
        }

        // Configure child task's thread-local to point to our request context.
        pub(crate) fn before_poll<P>(raw_ctx: *const ())
        where
            P: PrioritySelector,
        {
            // SAFETY: the hook holds one ref-count to the request context.
            let ctx = unsafe { &*(raw_ctx as *const P::ParentContext) };
            crate::masa::context::server::set_parent_ctx::<P>(ctx);
        }

        // Remove request context from our thread local to avoid exposing it
        // to another task (and mislead another task to think they have a parent rpc).
        pub(crate) fn after_poll<P>(_raw_ctx: *const ())
        where
            P: PrioritySelector,
        {
            crate::masa::context::server::reset_parent_ctx::<P>();
        }
    }
}
