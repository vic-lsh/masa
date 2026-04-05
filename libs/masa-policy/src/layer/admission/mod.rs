// Admission control layers — compile-time selected.
//
// - `predictive` (feature `ac_pred`): goodput-tracking token-bucket AC.
// - `rajomon` (feature `ac_rajomon`): token-based AC with price signals.
// - `NoopLayer`: when neither is enabled, compiles away to nothing.
//
// `ac_pred` and `ac_rajomon` are mutually exclusive (two admission
// controllers cannot coexist). Estimation (latency tracking, deadline
// tightening, feasibility checks) is handled by the separate
// `EstimationLayer` and does not conflict with either AC layer.

#[cfg(feature = "ac_pred")]
pub(crate) mod predictive;

#[cfg(feature = "ac_rajomon")]
pub mod rajomon;

// ── Mutual exclusion ────────────────────────────────────────────────────

#[cfg(all(feature = "ac_pred", feature = "ac_rajomon"))]
compile_error!(
    "Features `ac_pred` and `ac_rajomon` are mutually exclusive. Use one admission controller at a time."
);

// ── Compile-time admission layer selection ──────────────────────────────

#[cfg(feature = "ac_pred")]
pub(crate) use predictive::PredAdmissionLayer as AdmissionLayer;

#[cfg(all(feature = "ac_rajomon", not(feature = "ac_pred")))]
pub(crate) use rajomon::RajomonLayer as AdmissionLayer;

#[cfg(not(any(feature = "ac_pred", feature = "ac_rajomon")))]
pub(crate) use self::noop::NoopLayer as AdmissionLayer;

// ── Noop layer (inline) ─────────────────────────────────────────────────

#[cfg(not(any(feature = "ac_pred", feature = "ac_rajomon")))]
mod noop {
    use masa_core::Context;
    use tonic_core::CowGrpcMethod;

    use super::super::{Layer, LayerChild, LayerServer};

    #[derive(Debug)]
    pub(crate) struct NoopServer;

    impl LayerServer for NoopServer {
        fn new() -> Self {
            Self
        }
    }

    #[derive(Debug)]
    pub(crate) struct NoopLayer;

    impl Layer for NoopLayer {
        type Server = NoopServer;
        type Child = NoopChild;

        fn new(_method: &CowGrpcMethod, _server: &NoopServer, _ctx: &mut Context) -> Self {
            Self
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct NoopChild;

    impl LayerChild for NoopChild {
        fn new() -> Self {
            Self
        }
    }
}
