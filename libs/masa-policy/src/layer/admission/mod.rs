// Admission control layers — mutually exclusive, compile-time selected.
//
// - `predictive` (feature `estimator`): latency estimation + predictive AC.
// - `rajomon` (feature `ac_rajomon`): token-based AC with price signals.
// - `NoopLayer`: when neither is enabled, compiles away to nothing.

#[cfg(feature = "estimator")]
pub(crate) mod predictive;

#[cfg(feature = "ac_rajomon")]
pub mod rajomon;

// ── Mutual exclusion ────────────────────────────────────────────────────

#[cfg(all(feature = "estimator", feature = "ac_rajomon"))]
compile_error!(
    "Features `estimator` and `ac_rajomon` are mutually exclusive. Use one layer at a time."
);

// ── Compile-time policy layer selection ─────────────────────────────────

#[cfg(feature = "ac_rajomon")]
pub(crate) use rajomon::RajomonLayer as PolicyLayer;

#[cfg(all(feature = "estimator", not(feature = "ac_rajomon")))]
pub(crate) use predictive::PredAdmissionLayer as PolicyLayer;

#[cfg(not(any(feature = "estimator", feature = "ac_rajomon")))]
pub(crate) use self::noop::NoopLayer as PolicyLayer;

// ── Noop layer (inline) ─────────────────────────────────────────────────

#[cfg(not(any(feature = "estimator", feature = "ac_rajomon")))]
mod noop {
    // No-op layer — used when neither `estimator` nor `ac_rajomon` is enabled.
    //
    // All methods use the default no-op implementations from the trait, so
    // they compile away entirely.

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

        // All other methods use default no-op impls from the trait.
    }

    #[derive(Debug, Clone)]
    pub(crate) struct NoopChild;

    impl LayerChild for NoopChild {
        fn new() -> Self {
            Self
        }
    }
}
