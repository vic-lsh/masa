thread_local! {
    // Thread-local storing the context of a parent RPC.
    //
    // Logically, this is set by the server task handling a user request. Child
    // RPCs, either done on the task originally handling the request, or done on
    // a newly spawned task, will see this parent context. This way, the child
    // RPC can establish the correct parent-child relationship.
    //
    // Because this is a std::thread_local and not one made for an async runtime,
    // this field is set only during a future is polled. code-gen tonic ensures
    // that all the futures that should have visibility into this parent_ctx are
    // marked. Then, when these futures are executed, this thread local is set
    // before they're polled and reset after a poll completes.
    static PARENT_CTX: std::cell::Cell<*const ()> =
                        std::cell::Cell::new(core::ptr::null());

}

///
pub mod client {

    use crate::masa::{ClientStubHooks, RequestHandlerHooks, ServerHooks};

    /// SAFETY:
    /// - Caller must ensure that the generic P correct: the same P is used in
    /// setting the parent context as well as in retrieving it.
    pub unsafe fn get_parent_ctx<
        'a,
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    >() -> Option<&'a P> {
        let p = super::PARENT_CTX.get();
        let parent_ctx = p as *const P;
        parent_ctx.as_ref()
    }
}

///
pub mod server {
    use crate::masa::{ClientStubHooks, RequestHandlerHooks, ServerHooks};

    ///
    pub fn set_parent_ctx<'a, S: ServerHooks, C: ClientStubHooks, P: RequestHandlerHooks<C, S>>(
        parent_ctx: &'a P,
    ) {
        super::PARENT_CTX.replace(parent_ctx as *const P as *const ());
    }

    ///
    pub fn reset_parent_ctx<
        'a,
        S: ServerHooks,
        C: ClientStubHooks,
        P: RequestHandlerHooks<C, S>,
    >() {
        super::PARENT_CTX.replace(core::ptr::null());
    }
}
