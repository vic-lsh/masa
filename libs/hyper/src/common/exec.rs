use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
use crate::body::Body;
#[cfg(feature = "server")]
use crate::body::HttpBody;
#[cfg(all(feature = "http2", feature = "server"))]
use crate::proto::h2::server::H2Stream;
use crate::rt::Executor;
#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
use crate::server::server::{new_svc::NewSvcTask, Watcher};
#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
use crate::service::HttpService;
#[cfg(feature = "server")]
use http::HeaderMap;
#[cfg(feature = "server")]
use masa_core::Context as MasaContext;
use tokio::task::TaskPriority;

#[cfg(feature = "server")]
pub trait ConnStreamExec<F, B: HttpBody>: Clone {
    fn h2_stream_priority(&self, _headers: &HeaderMap) -> TaskPriority {
        TaskPriority::infra()
    }

    fn execute_h2stream_with_prio(&mut self, fut: H2Stream<F, B>, prio: TaskPriority);
}

#[cfg(feature = "server")]
fn masa_priority_from_headers(headers: &HeaderMap) -> TaskPriority {
    let ctx = headers
        .get(masa_core::MASA_CONTEXT_HEADER)
        .unwrap_or_else(|| panic!("{}", masa_core::MISSING_CONTEXT_HEADER_MESSAGE));
    let ctx_str = ctx.to_str().unwrap_or_else(|err| {
        panic!(
            "{}",
            masa_core::invalid_context_header_metadata_message(err)
        )
    });

    TaskPriority::new(MasaContext::from_header_string(ctx_str).prio_hint().value())
}

#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
pub trait NewSvcExec<I, N, S: HttpService<Body>, E, W: Watcher<I, S, E>>: Clone {
    fn execute_new_svc(&mut self, fut: NewSvcTask<I, N, S, E, W>);
}

pub(crate) type BoxSendFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Specify what executor to use.
// Either the user provides an executor for background tasks, or we use
// `tokio::spawn`.
#[derive(Clone)]
pub enum Exec {
    /// Use tokio by default.
    Default,
    /// Use masa-specific runtime.
    Masa,
    /// Use custom executor.
    Executor(Arc<dyn Executor<BoxSendFuture> + Send + Sync>),
}

// ===== impl Exec =====

impl Exec {
    #[cfg(feature = "server")]
    pub(crate) fn h2_stream_priority(&self, headers: &HeaderMap) -> TaskPriority {
        match self {
            Exec::Masa => masa_priority_from_headers(headers),
            Exec::Default | Exec::Executor(_) => TaskPriority::infra(),
        }
    }

    pub(crate) fn execute<F>(&self, fut: F, prio: TaskPriority)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        match *self {
            Exec::Default => {
                #[cfg(feature = "tcp")]
                {
                    tokio::task::spawn(fut);
                }
                #[cfg(not(feature = "tcp"))]
                {
                    // If no runtime, we need an executor!
                    panic!("executor must be set")
                }
            }
            Exec::Masa => {
                {
                    tokio::task::spawn_with_prio(fut, prio);
                }
                #[cfg(any())]
                {
                    async_executor::spawn_with_prio(fut, prio)
                        .fallible()
                        .detach();
                }
            }
            Exec::Executor(ref e) => {
                e.execute(Box::pin(fut), prio);
            }
        }
    }
}

impl fmt::Debug for Exec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exec").finish()
    }
}

#[cfg(feature = "server")]
impl<F, B> ConnStreamExec<F, B> for Exec
where
    H2Stream<F, B>: Future<Output = ()> + Send + 'static,
    B: HttpBody,
{
    fn h2_stream_priority(&self, headers: &HeaderMap) -> TaskPriority {
        Exec::h2_stream_priority(self, headers)
    }

    fn execute_h2stream_with_prio(&mut self, fut: H2Stream<F, B>, prio: TaskPriority) {
        self.execute(fut, prio)
    }
}

#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
impl<I, N, S, E, W> NewSvcExec<I, N, S, E, W> for Exec
where
    NewSvcTask<I, N, S, E, W>: Future<Output = ()> + Send + 'static,
    S: HttpService<Body>,
    W: Watcher<I, S, E>,
{
    fn execute_new_svc(&mut self, fut: NewSvcTask<I, N, S, E, W>) {
        self.execute(fut, TaskPriority::infra())
    }
}

// ==== impl Executor =====

#[cfg(feature = "server")]
impl<E, F, B> ConnStreamExec<F, B> for E
where
    E: Executor<H2Stream<F, B>> + Clone,
    H2Stream<F, B>: Future<Output = ()>,
    B: HttpBody,
{
    fn execute_h2stream_with_prio(&mut self, fut: H2Stream<F, B>, prio: TaskPriority) {
        self.execute(fut, prio)
    }
}

#[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
impl<I, N, S, E, W> NewSvcExec<I, N, S, E, W> for E
where
    E: Executor<NewSvcTask<I, N, S, E, W>> + Clone,
    NewSvcTask<I, N, S, E, W>: Future<Output = ()>,
    S: HttpService<Body>,
    W: Watcher<I, S, E>,
{
    fn execute_new_svc(&mut self, fut: NewSvcTask<I, N, S, E, W>) {
        self.execute(fut, TaskPriority::infra())
    }
}

#[cfg(all(test, feature = "server"))]
mod masa_context_tests {
    use super::*;
    use http::HeaderValue;
    use masa_core::{ContextBuilder, PriorityHint};

    #[test]
    fn default_executor_ignores_missing_context() {
        let headers = HeaderMap::new();

        assert_eq!(
            Exec::Default.h2_stream_priority(&headers),
            TaskPriority::infra()
        );
    }

    #[test]
    #[should_panic(expected = "missing MASA context header `ctx`")]
    fn masa_executor_requires_context() {
        let headers = HeaderMap::new();

        let _ = Exec::Masa.h2_stream_priority(&headers);
    }

    #[test]
    #[should_panic(expected = "invalid MASA context header `ctx`: invalid ASCII/metadata")]
    fn masa_executor_panics_on_invalid_ascii_context() {
        let mut headers = HeaderMap::new();
        headers.insert(
            masa_core::MASA_CONTEXT_HEADER,
            HeaderValue::from_bytes(b"\xff").unwrap(),
        );

        let _ = Exec::Masa.h2_stream_priority(&headers);
    }

    #[test]
    fn masa_executor_uses_context_priority_hint() {
        let ctx = ContextBuilder::new("test.Service", 9)
            .slo(100)
            .gateway_entry(10)
            .deadline(110)
            .prio_hint(PriorityHint::new(42))
            .build();
        let mut headers = HeaderMap::new();
        headers.insert(
            masa_core::MASA_CONTEXT_HEADER,
            HeaderValue::from_str(&ctx.to_header_string()).unwrap(),
        );

        assert_eq!(
            Exec::Masa.h2_stream_priority(&headers),
            TaskPriority::new(42)
        );
    }
}

// If http2 is not enable, we just have a stub here, so that the trait bounds
// that *would* have been needed are still checked. Why?
//
// Because enabling `http2` shouldn't suddenly add new trait bounds that cause
// a compilation error.
#[cfg(not(feature = "http2"))]
#[allow(missing_debug_implementations)]
pub struct H2Stream<F, B>(std::marker::PhantomData<(F, B)>);

#[cfg(not(feature = "http2"))]
impl<F, B, E> Future for H2Stream<F, B>
where
    F: Future<Output = Result<http::Response<B>, E>>,
    B: crate::body::HttpBody,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        unreachable!()
    }
}
