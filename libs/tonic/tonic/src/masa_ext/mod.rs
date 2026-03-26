// Re-export core hook traits and types from tonic-core.
pub use tonic_core::masa_ext::{
    noop, resolve_method_name_from_http, resolve_method_name_from_request, ClientHooks, MasaHooks,
    ParentHooks, ServerHooks, METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER,
};

// Re-export context extension traits and helpers from masa-policy.
pub use masa_policy::context_ext::{
    get_masa_context_from_metadata, read_context, set_masa_context_in_metadata, MasaRequestExt,
    MasaResponseExt, MasaStatusExt, MASA_CONTEXT_HEADER,
};

// Tonic-specific runtime glue (depends on tokio PollHook).
pub mod runtime;
mod thread_local;
pub use thread_local::{client, server};

/// Default hooks type, selected at compile time by feature flags.
///
/// - No scheduling features: `NoopMasaHooks` (zero overhead).
/// - Any scheduling feature (`sched_fifo`, `sched_slo`, `sched_tailclipper`):
///   `masa_policy::StandardHooks` with full scheduling hooks.
#[cfg(not(any(
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper"
)))]
pub type DefaultMasaHooks = tonic_core::masa_ext::noop::NoopMasaHooks;

/// Default hooks type, selected at compile time by feature flags.
#[cfg(any(
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper"
))]
pub type DefaultMasaHooks = masa_policy::StandardHooks;
