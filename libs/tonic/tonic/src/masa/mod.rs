//! Masa hook surface for tonic internals and generated code.
//!
//! Policy hook selection and concrete context extension traits are owned by
//! `masa`.

mod future;
mod hooks;
mod thread_local;

/// No-op Masa hooks implementation for when no scheduling features are enabled.
pub mod noop;

/// Tokio poll-hook bridge for propagating parent context to child tasks.
#[cfg(feature = "transport")]
pub mod runtime;

pub use future::{Abortable, AbortableFuture, AbortableFutureBuilder, AfterPollFn, BeforePollFn};
pub use hooks::{
    resolve_method_name_from_http, resolve_method_name_from_request, ClientHooks, Hooks,
    ParentHooks, ServerHooks, METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER,
};
pub use thread_local::{client, server};
