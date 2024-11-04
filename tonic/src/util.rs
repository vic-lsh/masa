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

/// Struct for defining hook points before and after polling a future.
#[derive(Debug)]
pub struct HookedFuture<F, Pre, Post> {
    inner: F,
    pre_hook: Option<Pre>,
    post_hook: Option<Post>,
}

/// Hook invoked before a future is polled.
///
/// The function can provide an output value. If provided, the future will never
/// be polled again, and the output value is immediately returned.
pub trait PreHookBound<F: Future> = Fn() -> Option<F::Output>;

/// Hook invoked after a future is polled.
///
/// The function can optionally provide an output value. If provided, this
/// output value will be the one returned, even if the future is already ready.
pub trait PostHookBound<F: Future> = Fn(&Poll<F::Output>) -> Option<F::Output>;

/// Struct for building a HookedFuture.
#[derive(Debug)]
pub struct HookedFutureBuilder<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
    _marker: PhantomData<(Pre, Post)>,
}

impl<F: Future> HookedFutureBuilder<F, (), ()> {
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

impl<F, Pre, Post> HookedFutureBuilder<F, Pre, Post>
where
    F: Future,
{
    /// Define hook point before polling.
    pub fn pre_hook<NewPre: PreHookBound<F>>(
        self,
        hook: NewPre,
    ) -> HookedFutureBuilder<F, NewPre, Post> {
        HookedFutureBuilder {
            inner: self.inner,
            before_poll: Some(hook),
            after_poll: self.after_poll,
            _marker: PhantomData,
        }
    }
}

impl<F, Pre, Post> HookedFutureBuilder<F, Pre, Post>
where
    F: Future,
{
    /// Define hook point after polling.
    pub fn post_hook<NewPost: PostHookBound<F>>(
        self,
        hook: NewPost,
    ) -> HookedFutureBuilder<F, Pre, NewPost> {
        HookedFutureBuilder {
            inner: self.inner,
            before_poll: self.before_poll,
            after_poll: Some(hook),
            _marker: PhantomData,
        }
    }
}

impl<F, Pre, Post> HookedFutureBuilder<F, Pre, Post>
where
    F: Future,
    Pre: PreHookBound<F>,
    Post: PostHookBound<F>,
{
    ///
    pub fn build(self) -> HookedFuture<F, Pre, Post> {
        HookedFuture {
            inner: self.inner,
            pre_hook: self.before_poll,
            post_hook: self.after_poll,
        }
    }
}

impl<F, Pre, Post> Future for HookedFuture<F, Pre, Post>
where
    F: Future,
    Pre: PreHookBound<F>,
    Post: PostHookBound<F>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We're not moving any fields out of self
        let this = unsafe { self.get_unchecked_mut() };

        // Call the pre-hook if it exists
        if let Some(pre_hook) = &this.pre_hook {
            if let Some(alt_output) = pre_hook() {
                return Poll::Ready(alt_output);
            }
        }

        // Poll the inner future
        // SAFETY: We're not moving the future, just polling it
        let poll_result = unsafe { Pin::new_unchecked(&mut this.inner) }.poll(cx);

        // Call the post-hook if it exists
        if let Some(post_hook) = &this.post_hook {
            if let Some(alt_output) = post_hook(&poll_result) {
                return Poll::Ready(alt_output);
            }
        }

        poll_result
    }
}

/// Trait to add the `hook` method to futures
pub trait Hookable: Sized + Future {
    /// Start building a HookedFuture.
    fn hook(self) -> HookedFutureBuilder<Self, (), ()>;
}

impl<F: Future> Hookable for F {
    fn hook(self) -> HookedFutureBuilder<Self, (), ()> {
        HookedFutureBuilder::new(self)
    }
}
