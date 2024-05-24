//! Runtime components
//!
//! By default, hyper includes the [tokio](https://tokio.rs) runtime.
//!
//! If the `runtime` feature is disabled, the types in this module can be used
//! to plug in other runtimes.

/// An executor of futures.
pub trait Executor<Fut> {
    /// Place the future into the executor with a deadline hint.
    fn execute(&self, fut: Fut, ddl: DeadlineHint);
}

pub use crate::common::deadline::DeadlineHint;
#[cfg(any(feature = "http1", feature = "http2", feature = "server"))]
pub use crate::common::exec::Exec;
