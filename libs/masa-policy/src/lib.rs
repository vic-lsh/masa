// Masa scheduling policy implementations.
//
// This crate contains the scheduling, estimation, and admission control
// implementations for Masa. It depends on `tonic` for gRPC boundary types and
// hook traits. Tonic does not depend on this crate; `masa::DefaultHooks` selects
// policy hooks when scheduling features are enabled.

/// Masa context extension traits and helpers.
pub mod context_ext;
mod hooks;
pub(crate) mod layer;
/// Runtime-configurable policy parameters loaded from policy_param.json.
pub mod policy_params;
/// Method registry for mapping service/method strings to IDs.
pub mod registry;

pub use context_ext::{
    get_masa_context_from_metadata, get_method_name_override_from_headers,
    get_method_name_override_from_metadata, get_service_name_override_from_headers,
    get_service_name_override_from_metadata, read_context, read_context_from_headers,
    read_priority_from_headers, set_masa_context_in_metadata, set_method_name_override_in_headers,
    set_service_name_override_in_headers, MasaRequestExt, MasaResponseExt, MasaStatusExt,
    MASA_CONTEXT_HEADER,
};
pub use hooks::{ChildContext, ParentContext, PolicyHooks, ServerContext};
pub use registry::{MethodId, MethodRegistry};

pub use policy_params::PolicyParams;

// Re-export Rajomon public items when the feature is enabled.
#[cfg(all(feature = "ac_rajomon", not(feature = "ac_pred")))]
pub use layer::admission::rajomon::{
    ClientTokenBucket, RajomonSharedState, CLIENT_TOKEN_BUCKET, RAJOMON_STATE,
};
