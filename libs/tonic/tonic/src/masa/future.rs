use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

/// Future wrapper that can return early from hooks before or after polling.
#[derive(Debug)]
pub(crate) struct AbortableFuture<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
}

/// Hook invoked before a future is polled.
///
/// The function can provide an output value. If provided, the future will never
/// be polled again, and the output value is immediately returned.
pub(crate) trait BeforePollFn<F: Future> = Fn() -> Option<F::Output>;

/// Hook invoked after a future is polled.
///
/// The function can optionally provide an output value. If provided, this
/// output value will be the one returned, even if the future is already ready.
pub(crate) trait AfterPollFn<F: Future> = Fn(&Poll<F::Output>) -> Option<F::Output>;

/// Builder for [`AbortableFuture`].
#[derive(Debug)]
pub(crate) struct AbortableFutureBuilder<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
    _marker: PhantomData<(Pre, Post)>,
}

impl<F: Future> AbortableFutureBuilder<F, (), ()> {
    /// Start constructing an [`AbortableFuture`].
    pub(crate) fn new(future: F) -> Self {
        Self {
            inner: future,
            before_poll: None,
            after_poll: None,
            _marker: PhantomData,
        }
    }
}

impl<F, Pre, Post> AbortableFutureBuilder<F, Pre, Post>
where
    F: Future,
{
    /// Define the hook point before polling.
    pub(crate) fn before_poll<NewPre: BeforePollFn<F>>(
        self,
        hook: NewPre,
    ) -> AbortableFutureBuilder<F, NewPre, Post> {
        AbortableFutureBuilder {
            inner: self.inner,
            before_poll: Some(hook),
            after_poll: self.after_poll,
            _marker: PhantomData,
        }
    }
}

impl<F, Pre, Post> AbortableFutureBuilder<F, Pre, Post>
where
    F: Future,
{
    /// Define the hook point after polling.
    pub(crate) fn after_poll<NewPost: AfterPollFn<F>>(
        self,
        hook: NewPost,
    ) -> AbortableFutureBuilder<F, Pre, NewPost> {
        AbortableFutureBuilder {
            inner: self.inner,
            before_poll: self.before_poll,
            after_poll: Some(hook),
            _marker: PhantomData,
        }
    }
}

impl<F, Pre, Post> AbortableFutureBuilder<F, Pre, Post>
where
    F: Future,
    Pre: BeforePollFn<F>,
    Post: AfterPollFn<F>,
{
    /// Finish constructing the wrapped future.
    pub(crate) fn build(self) -> AbortableFuture<F, Pre, Post> {
        AbortableFuture {
            inner: self.inner,
            before_poll: self.before_poll,
            after_poll: self.after_poll,
        }
    }
}

impl<F, Pre, Post> Future for AbortableFuture<F, Pre, Post>
where
    F: Future,
    Pre: BeforePollFn<F>,
    Post: AfterPollFn<F>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: this projection never moves any field out of `self`.
        let this = unsafe { self.get_unchecked_mut() };

        if let Some(pre_hook) = &this.before_poll {
            if let Some(alt_output) = pre_hook() {
                return Poll::Ready(alt_output);
            }
        }

        // SAFETY: the inner future remains pinned in place inside `self`.
        let poll_result = unsafe { Pin::new_unchecked(&mut this.inner) }.poll(cx);

        if let Some(post_hook) = &this.after_poll {
            if let Some(alt_output) = post_hook(&poll_result) {
                return Poll::Ready(alt_output);
            }
        }

        poll_result
    }
}

/// Extension trait for adding abort hook points to a future.
pub(crate) trait Abortable: Sized + Future {
    /// Start building an [`AbortableFuture`].
    fn abortable(self) -> AbortableFutureBuilder<Self, (), ()>;
}

impl<F: Future> Abortable for F {
    fn abortable(self) -> AbortableFutureBuilder<Self, (), ()> {
        AbortableFutureBuilder::new(self)
    }
}
