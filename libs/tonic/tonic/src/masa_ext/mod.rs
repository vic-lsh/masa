//! Masa compatibility surface for tonic users.
//!
//! Hook traits come from `masa-tonic-core`. Policy hook selection and concrete
//! context extension traits are owned by `masa`.

// Re-export core hook traits and types from masa-tonic-core.
pub use masa_tonic_core::{
    client, noop, resolve_method_name_from_http, resolve_method_name_from_request, runtime, server,
    ClientHooks, Hooks, ParentHooks, ServerHooks, METHOD_NAME_OVERRIDE_HEADER,
    SERVICE_NAME_OVERRIDE_HEADER,
};
