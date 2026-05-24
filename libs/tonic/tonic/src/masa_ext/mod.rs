//! Masa compatibility surface for tonic users.
//!
//! Hook traits come from `masa-tonic-core`, while metadata context helpers and
//! extension traits are implemented in `masa-policy` and reexported here so
//! existing `tonic::masa_ext::*` imports keep working. Policy hook selection is
//! owned by `masa::DefaultHooks`.

// Re-export core hook traits and types from masa-tonic-core.
pub use masa_tonic_core::{
    client, noop, resolve_method_name_from_http, resolve_method_name_from_request, runtime, server,
    ClientHooks, Hooks, ParentHooks, ServerHooks, METHOD_NAME_OVERRIDE_HEADER,
    SERVICE_NAME_OVERRIDE_HEADER,
};

// Re-export context extension traits and helpers from masa-policy.
pub use masa_policy::context_ext::{
    get_masa_context_from_metadata, read_context, read_context_from_headers,
    set_masa_context_in_metadata, MasaRequestExt, MasaResponseExt, MasaStatusExt,
    MASA_CONTEXT_HEADER,
};
