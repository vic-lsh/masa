// Masa scheduling policy implementations.
//
// This crate contains the scheduling, estimation, and admission control
// implementations for Masa. It depends on `tonic` for gRPC types and
// hook traits, but tonic does NOT depend on this crate — keeping the
// policy logic modular and decoupled from the gRPC framework.

#[allow(missing_docs)]
pub mod ac;
mod base;
mod common;
#[cfg(feature = "est")]
pub(crate) mod est;
#[cfg(feature = "est")]
mod pred_sched;
/// Method registry for mapping service/method strings to IDs.
pub mod registry;
mod standard;

pub use registry::MethodRegistry;
pub use standard::{ChildContext, ParentContext, ServerContext, StandardHooks};
