std::thread_local! {
    // Thread-local storing the context of a parent RPC.
    //
    // Logically, this is set by the server task handling a user request. Child
    // RPCs, either done on the task originally handling the request, or done on
    // a newly spawned task, will see this parent context. This way, the child
    // RPC can establish the correct parent-child relationship.
    //
    // Because this is a std::thread_local and not one made for an async runtime,
    // this field is set only while a future is polled. Generated tonic code
    // ensures that all futures needing parent_ctx visibility are marked. Then,
    // when these futures execute, this thread local is set before they are
    // polled and reset after a poll completes.
    static PARENT_CTX: std::cell::Cell<*const ()> =
        std::cell::Cell::new(core::ptr::null());
}

/// Client-side access to the parent RPC context visible to child RPCs.
pub mod client {
    use super::super::Hooks;

    /// Return the parent RPC context currently visible to this task.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the same hook type parameter `M` is used
    /// when setting and retrieving the parent context.
    pub unsafe fn get_parent_ctx<'a, M: Hooks>() -> Option<&'a M::ParentContext> {
        let p = super::PARENT_CTX.get();
        let parent_ctx = p as *const M::ParentContext;
        unsafe { parent_ctx.as_ref() }
    }
}

/// Server-side management of the parent RPC context around handler polling.
pub mod server {
    use super::super::Hooks;

    /// Set the parent context visible to child RPCs in this task.
    ///
    /// This is an internal API exposed only for generated tonic code and the
    /// Tokio child task poll-hook bridge.
    pub fn set_parent_ctx<M>(parent_ctx: &M::ParentContext)
    where
        M: Hooks,
    {
        super::PARENT_CTX.replace(parent_ctx as *const M::ParentContext as *const ());
    }

    /// Clear the parent context visible to child RPCs in this task.
    ///
    /// This is an internal API exposed only for generated tonic code and the
    /// Tokio child task poll-hook bridge.
    pub fn reset_parent_ctx<M>()
    where
        M: Hooks,
    {
        super::PARENT_CTX.replace(core::ptr::null());
    }
}
