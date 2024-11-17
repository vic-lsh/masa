//! Various utilities used throughout tonic.

// some combinations of features might cause things here not to be used
#![allow(dead_code)]

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

pub(crate) mod base64 {
    use base64::{
        alphabet,
        engine::{
            general_purpose::{GeneralPurpose, GeneralPurposeConfig},
            DecodePaddingMode,
        },
    };

    pub(crate) const STANDARD: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_encode_padding(true)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );

    pub(crate) const STANDARD_NO_PAD: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_encode_padding(false)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
}

/// Struct for defining abort hook points before and after polling a future.
#[derive(Debug)]
pub struct AbortableFuture<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
}

/// Hook invoked before a future is polled.
///
/// The function can provide an output value. If provided, the future will never
/// be polled again, and the output value is immediately returned.
pub trait BeforePollFn<F: Future> = Fn() -> Option<F::Output>;

/// Hook invoked after a future is polled.
///
/// The function can optionally provide an output value. If provided, this
/// output value will be the one returned, even if the future is already ready.
pub trait AfterPollFn<F: Future> = Fn(&Poll<F::Output>) -> Option<F::Output>;

/// Struct for building a HookedFuture.
#[derive(Debug)]
pub struct AbortableFutureBuilder<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
    _marker: PhantomData<(Pre, Post)>,
}

impl<F: Future> AbortableFutureBuilder<F, (), ()> {
    /// Start constructing a HookedFuture.
    pub fn new(future: F) -> Self {
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
    /// Define hook point before polling.
    pub fn before_poll<NewPre: BeforePollFn<F>>(
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
    /// Define hook point after polling.
    pub fn after_poll<NewPost: AfterPollFn<F>>(
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
    ///
    pub fn build(self) -> AbortableFuture<F, Pre, Post> {
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
        // SAFETY: We're not moving any fields out of self
        let this = unsafe { self.get_unchecked_mut() };

        // Call the pre-hook if it exists
        if let Some(pre_hook) = &this.before_poll {
            if let Some(alt_output) = pre_hook() {
                return Poll::Ready(alt_output);
            }
        }

        // Poll the inner future
        // SAFETY: We're not moving the future, just polling it
        let poll_result = unsafe { Pin::new_unchecked(&mut this.inner) }.poll(cx);

        // Call the post-hook if it exists
        if let Some(post_hook) = &this.after_poll {
            if let Some(alt_output) = post_hook(&poll_result) {
                return Poll::Ready(alt_output);
            }
        }

        poll_result
    }
}

/// Trait to add the abort behavior before and after polling a future.
pub trait Abortable: Sized + Future {
    /// Start building an AbortableFuture.
    fn abortable(self) -> AbortableFutureBuilder<Self, (), ()>;
}

impl<F: Future> Abortable for F {
    fn abortable(self) -> AbortableFutureBuilder<Self, (), ()> {
        AbortableFutureBuilder::new(self)
    }
}
