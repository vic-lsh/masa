// Masa scheduling policy implementations.
//
// This crate contains the scheduling, estimation, and admission control
// implementations for Masa. It depends on `tonic-core` for gRPC types and
// hook traits, and optionally `tonic` depends on this crate to wire up
// `DefaultMasaHooks`.

#[allow(missing_docs)]
pub mod ac;
mod base;
mod common;
/// Masa context extension traits and helpers (moved from tonic to break circular dep).
pub mod context_ext;
#[cfg(feature = "est")]
pub(crate) mod est;
#[cfg(feature = "est")]
mod pred_sched;
/// Method registry for mapping service/method strings to IDs.
pub mod registry;
mod standard;

pub use context_ext::{
    get_masa_context_from_metadata, read_context, set_masa_context_in_metadata, MasaRequestExt,
    MasaResponseExt, MasaStatusExt, MASA_CONTEXT_HEADER,
};
pub use registry::MethodRegistry;
pub use standard::{ChildContext, ParentContext, ServerContext, StandardHooks};
