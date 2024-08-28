//! Runtime components
//!
//! By default, hyper includes the [tokio](https://tokio.rs) runtime.
//!
//! If the `runtime` feature is disabled, the types in this module can be used
//! to plug in other runtimes.

use tonic_masa::DeadlineHint;

/// An executor of futures.
pub trait Executor<Fut> {
    /// Place a future with a deadline hint onto the executor.
    fn execute(&self, fut: Fut, ddl: DeadlineHint);
}

#[cfg(any(feature = "http1", feature = "http2", feature = "server"))]
pub use crate::common::exec::Exec;
