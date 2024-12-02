/// Hooks into a future's execution by running custom logic before and after
/// it is polled.
///
/// The hooks are provided with an argument pointing to a memory region,
/// dubbed `context` (the `ctx` field). `on_destroy` and `on_clone` functions
/// enable users to track and release resources associated with `ctx`.
///
/// This is a low-level struct. Use this struct with care.
/// See the documentation about each internal field for usage.
#[derive(Debug)]
pub struct RawPollHook {
    // Pointer that will be passed to the polling lifecycle hooks.
    // RawPollHook does not check for `ctx` pointer's validity.
    ctx: *const (),

    // Called when the struct is destroyed to let the user release resources
    // associated with `ctx`. Destruction can happen in two ways:
    //
    //  1) when the struct is dropped
    //  2) when the struct's field is reconfigured (via configure()), at which
    //     point the struct loses accesses to the prior `ctx`.
    //
    // This field must be set if `ctx` is non-null.
    on_destroy: Option<fn(*const ())>,

    // Called when the struct is cloned. `ctx` can be shared across multiple
    // RawPollHook instances, and this function is a place to increase ref-count.
    //
    // This field must be set if `ctx` is non-null.
    on_clone: Option<fn(*const ())>,

    // Invoked before the future is polled.
    before_poll: Option<fn(*const ())>,

    // Invoked after the future is polled.
    after_poll: Option<fn(*const ())>,
}

impl Default for RawPollHook {
    fn default() -> Self {
        Self {
            ctx: core::ptr::null(),
            on_destroy: None,
            on_clone: None,
            before_poll: None,
            after_poll: None,
        }
    }
}

impl Clone for RawPollHook {
    fn clone(&self) -> Self {
        if !self.ctx.is_null() {
            (self
                .on_clone
                .as_ref()
                .expect("on_clone must exist if ctx exists"))(self.ctx);
        }
        Self {
            ctx: self.ctx,
            on_destroy: self.on_destroy,
            on_clone: self.on_clone,
            before_poll: self.before_poll,
            after_poll: self.after_poll,
        }
    }
}

impl Drop for RawPollHook {
    fn drop(&mut self) {
        self.reset();
    }
}

impl RawPollHook {
    /// Creates a new `RawPollHook`.
    ///
    /// See the struct-level comments for usage.
    pub unsafe fn new(
        ctx: *const (),
        on_clone: Option<fn(*const ())>,
        on_destroy: Option<fn(*const ())>,
        before_poll: Option<fn(*const ())>,
        after_poll: Option<fn(*const ())>,
    ) -> Self {
        let mut h = RawPollHook::default();
        h.configure(ctx, on_clone, on_destroy, before_poll, after_poll);
        h
    }

    pub(crate) unsafe fn configure(
        &mut self,
        ctx: *const (),
        on_clone: Option<fn(*const ())>,
        on_destroy: Option<fn(*const ())>,
        before_poll: Option<fn(*const ())>,
        after_poll: Option<fn(*const ())>,
    ) {
        self.ctx = ctx;
        if !self.ctx.is_null() {
            assert!(on_destroy.is_some());
            self.on_destroy = on_destroy;
            assert!(on_clone.is_some());
            self.on_clone = on_clone;
        }
        self.before_poll = before_poll;
        self.after_poll = after_poll;
    }

    pub(crate) fn reset(&mut self) {
        if let Some(dtor) = &self.on_destroy {
            dtor(self.ctx);
        }
        self.on_destroy = None;
        self.ctx = core::ptr::null();
        self.before_poll = None;
        self.after_poll = None;
    }

    pub(crate) fn invoke_before_poll(&self) {
        if let Some(hook) = &self.before_poll {
            (hook)(self.ctx);
        }
    }

    pub(crate) fn invoke_after_poll(&self) {
        if let Some(hook) = &self.after_poll {
            (hook)(self.ctx);
        }
    }
}
