use alloc::boxed::Box;
use core::future::Future;
use core::task::Poll;

use crate::RawPollHook;

/// Trait used to define custom behavior before and after a future is called.
pub trait PollHook {
    /// Called before polling.
    fn before_poll(&self);
    /// Called after polling.
    fn after_poll(&self);
}

/// The main HookedFuture struct
#[allow(missing_debug_implementations)]
pub struct PollHookFuture<F> {
    inner: F,
    hook: RawPollHook,
}

impl<F> Future for PollHookFuture<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We're not moving any fields out of self
        let this = unsafe { self.get_unchecked_mut() };

        this.hook.invoke_before_poll();
        // Poll the inner future
        // SAFETY: We're not moving the future, just polling it
        let poll_result = unsafe { std::pin::Pin::new_unchecked(&mut this.inner) }.poll(cx);

        this.hook.invoke_after_poll();

        poll_result
    }
}

/// Trait to add the `hook` method to futures
pub trait WithPollHook: Sized + Future {
    ///
    fn with_poll_hook(self, hook: RawPollHook) -> PollHookFuture<Self>;
}

impl<F: Future> WithPollHook for F {
    fn with_poll_hook(self, hook: RawPollHook) -> PollHookFuture<F> {
        PollHookFuture { inner: self, hook }
    }
}
