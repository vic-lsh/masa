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

// The main HookedFuture struct
pub struct HookedFuture<F, Pre, Post> {
    inner: F,
    pre_hook: Option<Pre>,
    post_hook: Option<Post>,
}

// The Builder struct
pub struct HookedFutureBuilder<F, Pre, Post> {
    inner: F,
    before_poll: Option<Pre>,
    after_poll: Option<Post>,
    _marker: PhantomData<(Pre, Post)>,
}

impl<F: Future> HookedFutureBuilder<F, (), ()> {
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
    ///
    pub fn pre_hook<NewPre: Fn()>(self, hook: NewPre) -> HookedFutureBuilder<F, NewPre, Post> {
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
    ///
    pub fn post_hook<NewPost: Fn(&Poll<F::Output>)>(
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
    Pre: Fn(),
    Post: Fn(&Poll<F::Output>),
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
    Pre: Fn(),
    Post: Fn(&Poll<F::Output>),
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We're not moving any fields out of self
        let this = unsafe { self.get_unchecked_mut() };

        // Call the pre-hook if it exists
        if let Some(pre_hook) = &this.pre_hook {
            pre_hook();
        }

        // Poll the inner future
        // SAFETY: We're not moving the future, just polling it
        let poll_result = unsafe { Pin::new_unchecked(&mut this.inner) }.poll(cx);

        // Call the post-hook if it exists
        if let Some(post_hook) = &this.post_hook {
            post_hook(&poll_result);
        }

        poll_result
    }
}

// Trait to add the `hook` method to futures
pub trait Hookable: Sized + Future {
    fn hook(self) -> HookedFutureBuilder<Self, (), ()>;
}

impl<F: Future> Hookable for F {
    fn hook(self) -> HookedFutureBuilder<Self, (), ()> {
        HookedFutureBuilder::new(self)
    }
}
