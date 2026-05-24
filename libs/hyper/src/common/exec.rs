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
use http::HeaderMap;
use tokio::task::TaskPriority;

#[cfg(feature = "server")]
pub trait ConnStreamExec<F, B: HttpBody>: Clone {
    fn h2_stream_priority(&self, _headers: &HeaderMap) -> TaskPriority {
        TaskPriority::infra()
    }

    fn execute_h2stream_with_prio(&mut self, fut: H2Stream<F, B>, prio: TaskPriority);
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
    Masa(fn(&HeaderMap) -> TaskPriority),
    /// Use custom executor.
    Executor(Arc<dyn Executor<BoxSendFuture> + Send + Sync>),
}

// ===== impl Exec =====

impl Exec {
    #[cfg(feature = "server")]
    /// Use the Masa runtime with an HTTP/2 stream priority extractor.
    pub fn masa(h2_stream_priority: fn(&HeaderMap) -> TaskPriority) -> Self {
        Exec::Masa(h2_stream_priority)
    }

    #[cfg(feature = "server")]
    pub(crate) fn h2_stream_priority(&self, headers: &HeaderMap) -> TaskPriority {
        match self {
            Exec::Masa(extract) => extract(headers),
            Exec::Default | Exec::Executor(_) => TaskPriority::infra(),
        }
    }

    pub(crate) fn execute<F>(&self, fut: F)
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
            Exec::Masa(_) => {
                tokio::task::spawn(fut);
                #[cfg(any())]
                {
                    async_executor::spawn(fut).fallible().detach();
                }
            }
            Exec::Executor(ref e) => {
                e.execute(Box::pin(fut));
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
        match self {
            Exec::Masa(_) => {
                tokio::task::spawn_with_prio(fut, prio);
            }
            _ => self.execute(fut),
        }
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
        self.execute(fut)
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
        let _ = prio;
        self.execute(fut)
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
        self.execute(fut)
    }
}

#[cfg(all(test, feature = "server"))]
mod h2_priority_extractor_tests {
    use super::*;

    #[test]
    fn default_executor_uses_infra_priority() {
        let headers = HeaderMap::new();

        assert_eq!(
            Exec::Default.h2_stream_priority(&headers),
            TaskPriority::infra()
        );
    }

    #[test]
    fn masa_executor_uses_configured_priority_extractor() {
        let mut headers = HeaderMap::new();
        headers.insert("x-test-priority", "42".parse().unwrap());
        let exec = Exec::masa(|headers| {
            headers
                .get("x-test-priority")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
                .map(TaskPriority::new)
                .unwrap_or_else(TaskPriority::infra)
        });

        assert_eq!(exec.h2_stream_priority(&headers), TaskPriority::new(42));
    }

    #[test]
    #[should_panic(expected = "extractor panic")]
    fn masa_executor_preserves_extractor_panic_behavior() {
        let exec = Exec::masa(|_| panic!("extractor panic"));

        let _ = exec.h2_stream_priority(&HeaderMap::new());
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
