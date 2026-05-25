//! Masa hook surface for tonic internals and generated code.
//!
//! Policy hook selection and concrete context extension traits are owned by
//! `masa`.

pub(crate) mod future;
mod hooks;
mod thread_local;

/// No-op Masa hooks implementation for when no scheduling features are enabled.
pub mod noop;

/// Tokio poll-hook bridge for propagating parent context to child tasks.
#[cfg(feature = "transport")]
pub mod runtime;

pub use hooks::{ClientHooks, Hooks, ParentHooks, ServerHooks};
pub use thread_local::{client, server};
